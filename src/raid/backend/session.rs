use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use crate::backend::agent::{AgentMessage, UserMessage};

const SCHEMA_VERSION: i64 = 3;
const APPLICATION_ID: i64 = 0x5241_4944;
const MAX_TITLE_CHARS: usize = 80;
const MAX_SLUG_CHARS: usize = 48;
const MAX_PROJECT_SLUG_CHARS: usize = 96;
const CREATE_ATTEMPTS: usize = 100;
const DEFAULT_TITLE: &str = "New session";
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

const CREATE_SCHEMA: &str = "
CREATE TABLE entries (
    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    parent_id TEXT REFERENCES entries(id) DEFERRABLE INITIALLY DEFERRED,
    kind TEXT NOT NULL,
    role TEXT,
    payload_json TEXT NOT NULL CHECK(json_valid(payload_json)),
    created_at INTEGER NOT NULL
) STRICT;
CREATE INDEX entries_parent_id ON entries(parent_id);
CREATE INDEX entries_kind ON entries(kind);
CREATE TABLE session (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    id TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    title_generated INTEGER NOT NULL CHECK(title_generated IN (0, 1)),
    project_path TEXT NOT NULL,
    canonical_project_path TEXT NOT NULL,
    system_prompt_snapshot TEXT NOT NULL,
    initial_provider TEXT NOT NULL,
    initial_model TEXT NOT NULL,
    initial_api TEXT NOT NULL,
    current_provider TEXT NOT NULL,
    current_model TEXT NOT NULL,
    current_api TEXT NOT NULL,
    parent_session TEXT,
    last_entry_id TEXT REFERENCES entries(id) DEFERRABLE INITIALLY DEFERRED,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
) STRICT;";

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("could not access session files: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not use session database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("could not encode session data: {0}")]
    Json(#[from] serde_json::Error),
    #[error("session is already open in another Raid process: {0}")]
    Locked(PathBuf),
    #[error("session schema version {found} is newer than supported version {supported}")]
    NewerSchema { found: i64, supported: i64 },
    #[error("invalid session database: {0}")]
    Corrupt(String),
    #[error("could not allocate a unique session filename")]
    FilenameExhausted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetadata {
    pub id: String,
    pub title: String,
    pub title_generated: bool,
    pub project_path: PathBuf,
    pub canonical_project_path: PathBuf,
    pub system_prompt_snapshot: String,
    pub initial_provider: String,
    pub initial_model: String,
    pub initial_api: String,
    pub current_provider: String,
    pub current_model: String,
    pub current_api: String,
    pub parent_session: Option<PathBuf>,
    pub last_entry_id: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionEntry {
    pub sequence: u64,
    pub id: String,
    pub parent_id: Option<String>,
    pub kind: String,
    pub role: Option<String>,
    pub payload: Value,
    pub created_at: u64,
}

#[derive(Debug, Clone)]
pub struct SessionSnapshot {
    pub metadata: SessionMetadata,
    pub active_messages: Vec<AgentMessage>,
    pub display_messages: Vec<AgentMessage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub path: PathBuf,
    pub id: String,
    pub title: String,
    pub updated_at: u64,
    pub message_count: u64,
    pub current_provider: String,
    pub current_model: String,
    pub locked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionRecord {
    pub summary: String,
    pub first_kept_entry_id: Option<String>,
    pub tokens_before: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub retained_tail: Vec<AgentMessage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

pub struct SessionStore {
    connection: Option<Connection>,
    lock_file: Option<File>,
    path: PathBuf,
}

impl SessionStore {
    pub(crate) fn create_in(
        sessions_root: &Path,
        project_path: &Path,
        system_prompt: &str,
        provider: &str,
        model: &str,
        api: &str,
    ) -> Result<Self, SessionError> {
        let canonical_project_path = canonical_path(project_path);
        let project_dir = sessions_root.join(project_directory_name(&canonical_project_path));
        create_private_dir(&project_dir)?;
        let created_at = now_ms();

        for _ in 0..CREATE_ATTEMPTS {
            let id = new_id();
            let path = project_dir.join(format!("{}--{id}.db", session_slug(DEFAULT_TITLE)));
            match create_private_file(&path) {
                Ok(()) => {
                    let result = Self::initialize(
                        path.clone(),
                        &id,
                        project_path,
                        &canonical_project_path,
                        system_prompt,
                        provider,
                        model,
                        api,
                        created_at,
                    );
                    if result.is_err() {
                        let _ = std::fs::remove_file(path);
                    }
                    return result;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }

        Err(SessionError::FilenameExhausted)
    }

    #[allow(clippy::too_many_arguments)]
    fn initialize(
        path: PathBuf,
        id: &str,
        project_path: &Path,
        canonical_project_path: &Path,
        system_prompt: &str,
        provider: &str,
        model: &str,
        api: &str,
        created_at: u64,
    ) -> Result<Self, SessionError> {
        let lock_file = lock_session_file(&path)?;
        let mut connection = Connection::open(&path)?;
        configure_connection(&connection)?;
        let transaction = connection.transaction()?;
        transaction.execute_batch(CREATE_SCHEMA)?;
        transaction.execute(
            "INSERT INTO session (
                singleton, id, title, title_generated, project_path, canonical_project_path,
                system_prompt_snapshot, initial_provider, initial_model, current_provider,
                current_model, initial_api, current_api, parent_session, last_entry_id,
                created_at, updated_at
            ) VALUES (1, ?1, ?2, 0, ?3, ?4, ?5, ?6, ?7, ?6, ?7, ?8, ?8, NULL, NULL, ?9, ?9)",
            params![
                id,
                DEFAULT_TITLE,
                project_path.to_string_lossy(),
                canonical_project_path.to_string_lossy(),
                system_prompt,
                provider,
                model,
                api,
                sqlite_timestamp(created_at),
            ],
        )?;
        transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
        transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        transaction.commit()?;

        Ok(Self {
            connection: Some(connection),
            lock_file: Some(lock_file),
            path,
        })
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, SessionError> {
        let path = path.as_ref().to_path_buf();
        let lock_file = lock_session_file(&path)?;
        let mut connection = Connection::open(&path)?;
        configure_connection(&connection)?;
        migrate_if_needed(&mut connection)?;
        validate_database(&connection)?;
        Ok(Self {
            connection: Some(connection),
            lock_file: Some(lock_file),
            path,
        })
    }

    pub fn append_message(&mut self, message: &AgentMessage) -> Result<String, SessionError> {
        let payload = serde_json::to_value(message)?;
        self.append_entry("message", Some(message.role()), &payload, message_timestamp(message), None)
    }

    pub fn record_model_change(
        &mut self,
        provider: &str,
        model: &str,
        api: &str,
    ) -> Result<String, SessionError> {
        let timestamp = now_ms();
        let payload = json!({ "provider": provider, "model": model, "api": api });
        let connection = self.connection_mut()?;
        let transaction = connection.transaction()?;
        let id = append_entry_in_transaction(
            &transaction,
            "model_change",
            None,
            &payload,
            timestamp,
            None,
        )?;
        transaction.execute(
            "UPDATE session SET current_provider = ?1, current_model = ?2, current_api = ?3, updated_at = ?4 WHERE singleton = 1",
            params![provider, model, api, sqlite_timestamp(timestamp)],
        )?;
        transaction.commit()?;
        Ok(id)
    }

    pub fn append_compaction(&mut self, record: &CompactionRecord) -> Result<String, SessionError> {
        let mut stored = record.clone();
        if stored.first_kept_entry_id.is_none()
            && let Some(first_kept) = stored.retained_tail.first()
            && let Some(entry_id) = find_active_message_entry_id(self.connection_ref()?, first_kept)?
        {
            stored.first_kept_entry_id = Some(entry_id);
            stored.retained_tail.clear();
        }
        let payload = serde_json::to_value(stored)?;
        self.append_entry("compaction", None, &payload, now_ms(), None)
    }

    #[allow(dead_code)]
    pub fn append_branch_summary(
        &mut self,
        from_id: &str,
        summary: &str,
        details: Option<Value>,
    ) -> Result<String, SessionError> {
        let payload = json!({ "fromId": from_id, "summary": summary, "details": details });
        self.append_entry("branch_summary", None, &payload, now_ms(), None)
    }

    #[allow(dead_code)]
    pub fn set_leaf(&mut self, entry_id: Option<&str>) -> Result<(), SessionError> {
        let connection = self.connection_mut()?;
        if let Some(entry_id) = entry_id {
            let exists = connection
                .query_row("SELECT 1 FROM entries WHERE id = ?1", [entry_id], |_| Ok(()))
                .optional()?
                .is_some();
            if !exists {
                return Err(SessionError::Corrupt(format!("entry {entry_id} does not exist")));
            }
        }
        connection.execute(
            "UPDATE session SET last_entry_id = ?1 WHERE singleton = 1",
            [entry_id],
        )?;
        Ok(())
    }

    pub fn set_title(&mut self, title: &str, generated: bool) -> Result<(), SessionError> {
        let title = sanitize_title(title);
        let timestamp = now_ms();
        let connection = self.connection_mut()?;
        let transaction = connection.transaction()?;
        let payload = json!({ "title": title, "generated": generated });
        append_entry_in_transaction(
            &transaction,
            "session_info",
            None,
            &payload,
            timestamp,
            None,
        )?;
        transaction.execute(
            "UPDATE session SET title = ?1, title_generated = ?2, updated_at = ?3 WHERE singleton = 1",
            params![title, i64::from(generated), sqlite_timestamp(timestamp)],
        )?;
        transaction.commit()?;
        self.rename_for_title(&title)
    }

    pub fn snapshot(&self) -> Result<SessionSnapshot, SessionError> {
        load_snapshot(self.connection_ref()?)
    }

    pub fn metadata(&self) -> Result<SessionMetadata, SessionError> {
        load_metadata(self.connection_ref()?)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn current_title_is_replaceable(&self) -> Result<bool, SessionError> {
        let metadata = self.metadata()?;
        Ok(!metadata.title_generated && metadata.title == DEFAULT_TITLE)
    }

    fn append_entry(
        &mut self,
        kind: &str,
        role: Option<&str>,
        payload: &Value,
        created_at: u64,
        parent_id: Option<&str>,
    ) -> Result<String, SessionError> {
        let connection = self.connection_mut()?;
        let transaction = connection.transaction()?;
        let id = append_entry_in_transaction(
            &transaction,
            kind,
            role,
            payload,
            created_at,
            parent_id,
        )?;
        transaction.commit()?;
        Ok(id)
    }

    fn rename_for_title(&mut self, title: &str) -> Result<(), SessionError> {
        let metadata = self.metadata()?;
        let Some(parent) = self.path.parent() else {
            return Ok(());
        };
        let target = parent.join(format!("{}--{}.db", session_slug(title), metadata.id));
        if target == self.path {
            return Ok(());
        }

        let old_path = self.path.clone();
        self.close_handles()?;
        match std::fs::rename(&old_path, &target) {
            Ok(()) => match self.reopen_handles(&target) {
                Ok(()) => {
                    self.path = target;
                    Ok(())
                }
                Err(error) => {
                    let _ = std::fs::rename(&target, &old_path);
                    let _ = self.reopen_handles(&old_path);
                    Err(error)
                }
            },
            Err(error) => {
                self.reopen_handles(&old_path)?;
                Err(error.into())
            }
        }
    }

    fn close_handles(&mut self) -> Result<(), SessionError> {
        let connection = self
            .connection
            .take()
            .ok_or_else(|| SessionError::Corrupt("session connection is closed".into()))?;
        if let Err((connection, error)) = connection.close() {
            self.connection = Some(connection);
            return Err(error.into());
        }
        if let Some(file) = self.lock_file.take() {
            fs2::FileExt::unlock(&file)?;
        }
        Ok(())
    }

    fn reopen_handles(&mut self, path: &Path) -> Result<(), SessionError> {
        let lock_file = lock_session_file(path)?;
        let connection = Connection::open(path)?;
        configure_connection(&connection)?;
        self.lock_file = Some(lock_file);
        self.connection = Some(connection);
        Ok(())
    }

    fn connection_ref(&self) -> Result<&Connection, SessionError> {
        self.connection
            .as_ref()
            .ok_or_else(|| SessionError::Corrupt("session connection is closed".into()))
    }

    fn connection_mut(&mut self) -> Result<&mut Connection, SessionError> {
        self.connection
            .as_mut()
            .ok_or_else(|| SessionError::Corrupt("session connection is closed".into()))
    }
}

pub fn session_summaries(
    sessions_root: &Path,
    project_path: &Path,
) -> Result<Vec<SessionSummary>, SessionError> {
    let canonical_project_path = canonical_path(project_path);
    let project_dir = sessions_root.join(project_directory_name(&canonical_project_path));
    let entries = match std::fs::read_dir(project_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };

    let mut candidates = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("db"))
        .filter_map(|path| read_summary(&path).ok())
        .collect::<Vec<_>>();
    candidates.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
    Ok(candidates)
}

pub fn most_recent_session(
    sessions_root: &Path,
    project_path: &Path,
) -> Result<Option<PathBuf>, SessionError> {
    Ok(session_summaries(sessions_root, project_path)?
        .into_iter()
        .next()
        .map(|summary| summary.path))
}

pub fn delete_session(path: &Path) -> Result<(), SessionError> {
    trash::delete(path).map_err(|error| SessionError::Io(std::io::Error::other(error.to_string())))
}

fn read_summary(path: &Path) -> Result<SessionSummary, SessionError> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(SessionError::NewerSchema {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    if version == 1 {
        return legacy_summary(path, &connection);
    }
    if version != 2 && version != SCHEMA_VERSION {
        return Err(SessionError::Corrupt(format!(
            "unsupported session schema version {version}"
        )));
    }
    let mut summary = connection.query_row(
        "SELECT id, title, updated_at, current_provider, current_model FROM session WHERE singleton = 1",
        [],
        |row| {
            Ok(SessionSummary {
                path: path.to_path_buf(),
                id: row.get(0)?,
                title: row.get(1)?,
                updated_at: from_sqlite_timestamp(row.get(2)?),
                message_count: 0,
                current_provider: row.get(3)?,
                current_model: row.get(4)?,
                locked: false,
            })
        },
    )?;
    summary.message_count = connection.query_row(
        "SELECT COUNT(*) FROM entries WHERE kind = 'message'",
        [],
        |row| row.get::<_, i64>(0),
    )? as u64;
    summary.locked = is_session_locked(path);
    Ok(summary)
}

fn legacy_summary(path: &Path, connection: &Connection) -> Result<SessionSummary, SessionError> {
    connection
        .query_row(
            "SELECT id, title, updated_at, provider, model FROM session LIMIT 1",
            [],
            |row| {
                Ok(SessionSummary {
                    path: path.to_path_buf(),
                    id: row.get(0)?,
                    title: row.get(1)?,
                    updated_at: from_sqlite_timestamp(row.get(2)?),
                    message_count: connection
                        .query_row("SELECT COUNT(*) FROM messages", [], |count| count.get::<_, i64>(0))?
                        as u64,
                    current_provider: row.get(3)?,
                    current_model: row.get(4)?,
                    locked: is_session_locked(path),
                })
            },
        )
        .map_err(SessionError::from)
}

fn load_snapshot(connection: &Connection) -> Result<SessionSnapshot, SessionError> {
    let metadata = load_metadata(connection)?;
    let mut statement = connection.prepare(
        "SELECT sequence, id, parent_id, kind, role, payload_json, created_at FROM entries ORDER BY sequence",
    )?;
    let entries = statement
        .query_map([], |row| {
            let payload: String = row.get(5)?;
            let payload = serde_json::from_str(&payload).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    payload.len(),
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            Ok(SessionEntry {
                sequence: row.get::<_, i64>(0)? as u64,
                id: row.get(1)?,
                parent_id: row.get(2)?,
                kind: row.get(3)?,
                role: row.get(4)?,
                payload,
                created_at: from_sqlite_timestamp(row.get(6)?),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let active_messages = build_active_messages(&entries, metadata.last_entry_id.as_deref())?;
    let display_messages = build_display_messages(&entries, metadata.last_entry_id.as_deref())?;
    Ok(SessionSnapshot {
        metadata,
        active_messages,
        display_messages,
    })
}

fn load_metadata(connection: &Connection) -> Result<SessionMetadata, SessionError> {
    connection
        .query_row(
            "SELECT id, title, title_generated, project_path, canonical_project_path,
                    system_prompt_snapshot, initial_provider, initial_model, initial_api,
                    current_provider, current_model, current_api, parent_session, last_entry_id,
                    created_at, updated_at
             FROM session WHERE singleton = 1",
            [],
            |row| {
                Ok(SessionMetadata {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    title_generated: row.get::<_, i64>(2)? != 0,
                    project_path: PathBuf::from(row.get::<_, String>(3)?),
                    canonical_project_path: PathBuf::from(row.get::<_, String>(4)?),
                    system_prompt_snapshot: row.get(5)?,
                    initial_provider: row.get(6)?,
                    initial_model: row.get(7)?,
                    initial_api: row.get(8)?,
                    current_provider: row.get(9)?,
                    current_model: row.get(10)?,
                    current_api: row.get(11)?,
                    parent_session: row.get::<_, Option<String>>(12)?.map(PathBuf::from),
                    last_entry_id: row.get(13)?,
                    created_at: from_sqlite_timestamp(row.get(14)?),
                    updated_at: from_sqlite_timestamp(row.get(15)?),
                })
            },
        )
        .map_err(SessionError::from)
}

fn build_active_messages(
    entries: &[SessionEntry],
    leaf_id: Option<&str>,
) -> Result<Vec<AgentMessage>, SessionError> {
    let path = active_path(entries, leaf_id)?;

    let latest_compaction = path
        .iter()
        .enumerate()
        .rev()
        .find(|(_, entry)| entry.kind == "compaction");
    let mut messages = Vec::new();
    let mut start_index = 0;
    if let Some((compaction_index, entry)) = latest_compaction {
        let record: CompactionRecord = serde_json::from_value(entry.payload.clone())?;
        messages.push(AgentMessage::User(UserMessage::new(format!(
            "[Summary of earlier session context]\n{}",
            record.summary
        ))));
        if !record.retained_tail.is_empty() {
            messages.extend(record.retained_tail);
            start_index = compaction_index + 1;
        } else if let Some(first_kept) = record.first_kept_entry_id {
            start_index = path
                .iter()
                .position(|candidate| candidate.id == first_kept)
                .unwrap_or(compaction_index + 1);
        } else {
            start_index = compaction_index + 1;
        }
    }

    for entry in path.into_iter().skip(start_index) {
        match entry.kind.as_str() {
            "message" => messages.push(serde_json::from_value(entry.payload.clone())?),
            "branch_summary" => {
                if let Some(summary) = entry.payload.get("summary").and_then(Value::as_str) {
                    messages.push(AgentMessage::User(UserMessage::new(format!(
                        "[Summary of work from another branch]\n{summary}"
                    ))));
                }
            }
            _ => {}
        }
    }
    Ok(messages)
}

fn build_display_messages(
    entries: &[SessionEntry],
    leaf_id: Option<&str>,
) -> Result<Vec<AgentMessage>, SessionError> {
    active_path(entries, leaf_id)?
        .into_iter()
        .filter(|entry| entry.kind == "message")
        .map(|entry| serde_json::from_value(entry.payload.clone()).map_err(SessionError::from))
        .collect()
}

fn active_path<'a>(
    entries: &'a [SessionEntry],
    leaf_id: Option<&str>,
) -> Result<Vec<&'a SessionEntry>, SessionError> {
    let by_id = entries
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect::<HashMap<_, _>>();
    let mut path = Vec::new();
    let mut cursor = leaf_id;
    let mut visited = HashSet::new();
    while let Some(id) = cursor {
        if !visited.insert(id.to_string()) {
            return Err(SessionError::Corrupt("session entry tree contains a cycle".into()));
        }
        let entry = by_id
            .get(id)
            .copied()
            .ok_or_else(|| SessionError::Corrupt(format!("session entry {id} is missing")))?;
        path.push(entry);
        cursor = entry.parent_id.as_deref();
    }
    path.reverse();
    Ok(path)
}

fn find_active_message_entry_id(
    connection: &Connection,
    message: &AgentMessage,
) -> Result<Option<String>, SessionError> {
    let mut statement = connection.prepare(
        "WITH RECURSIVE active(id, parent_id, kind, payload_json, depth) AS (
             SELECT id, parent_id, kind, payload_json, 0
             FROM entries
             WHERE id = (SELECT last_entry_id FROM session WHERE singleton = 1)
             UNION ALL
             SELECT entry.id, entry.parent_id, entry.kind, entry.payload_json, active.depth + 1
             FROM entries AS entry
             JOIN active ON entry.id = active.parent_id
         )
         SELECT id, payload_json FROM active WHERE kind = 'message' ORDER BY depth",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (id, payload) = row?;
        let candidate: AgentMessage = serde_json::from_str(&payload)?;
        if candidate == *message {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

fn append_entry_in_transaction(
    transaction: &Transaction<'_>,
    kind: &str,
    role: Option<&str>,
    payload: &Value,
    created_at: u64,
    explicit_parent_id: Option<&str>,
) -> Result<String, SessionError> {
    let parent_id = match explicit_parent_id {
        Some(parent_id) => Some(parent_id.to_string()),
        None => transaction.query_row(
            "SELECT last_entry_id FROM session WHERE singleton = 1",
            [],
            |row| row.get::<_, Option<String>>(0),
        )?,
    };
    let id = new_id();
    transaction.execute(
        "INSERT INTO entries (id, parent_id, kind, role, payload_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            parent_id,
            kind,
            role,
            serde_json::to_string(payload)?,
            sqlite_timestamp(created_at),
        ],
    )?;
    transaction.execute(
        "UPDATE session SET last_entry_id = ?1, updated_at = ?2 WHERE singleton = 1",
        params![id, sqlite_timestamp(created_at)],
    )?;
    Ok(id)
}

fn configure_connection(connection: &Connection) -> Result<(), SessionError> {
    connection.busy_timeout(std::time::Duration::from_secs(2))?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.pragma_update(None, "journal_mode", "DELETE")?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    Ok(())
}

fn migrate_if_needed(connection: &mut Connection) -> Result<(), SessionError> {
    let version: i64 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > SCHEMA_VERSION {
        return Err(SessionError::NewerSchema {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    if version == SCHEMA_VERSION {
        return Ok(());
    }
    if version == 2 {
        let transaction = connection.transaction()?;
        transaction.execute_batch(
            "ALTER TABLE session ADD COLUMN initial_api TEXT NOT NULL DEFAULT 'openai-compatible';
             ALTER TABLE session ADD COLUMN current_api TEXT NOT NULL DEFAULT 'openai-compatible';",
        )?;
        transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        transaction.commit()?;
        return Ok(());
    }
    if version != 1 {
        return Err(SessionError::Corrupt(format!(
            "unsupported session schema version {version}"
        )));
    }

    let legacy_session = connection.query_row(
        "SELECT id, title, project_path, system_prompt, provider, model, created_at, updated_at FROM session LIMIT 1",
        [],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, i64>(7)?,
            ))
        },
    )?;
    let legacy_messages = {
        let mut statement = connection.prepare(
            "SELECT role, payload_json, created_at FROM messages ORDER BY sequence",
        )?;
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?
    };

    let transaction = connection.transaction()?;
    transaction.execute_batch(
        "ALTER TABLE session RENAME TO legacy_session;
         ALTER TABLE messages RENAME TO legacy_messages;",
    )?;
    transaction.execute_batch(CREATE_SCHEMA)?;
    let canonical = canonical_path(Path::new(&legacy_session.2));
    transaction.execute(
        "INSERT INTO session (
            singleton, id, title, title_generated, project_path, canonical_project_path,
            system_prompt_snapshot, initial_provider, initial_model, initial_api,
            current_provider, current_model, current_api, parent_session, last_entry_id,
            created_at, updated_at
        ) VALUES (1, ?1, ?2, 0, ?3, ?4, ?5, ?6, ?7, 'openai-compatible',
                  ?6, ?7, 'openai-compatible', NULL, NULL, ?8, ?9)",
        params![
            legacy_session.0,
            legacy_session.1,
            legacy_session.2,
            canonical.to_string_lossy(),
            legacy_session.3,
            legacy_session.4,
            legacy_session.5,
            legacy_session.6,
            legacy_session.7,
        ],
    )?;
    for (role, payload, created_at) in legacy_messages {
        let payload: Value = serde_json::from_str(&payload)?;
        append_entry_in_transaction(
            &transaction,
            "message",
            Some(&role),
            &payload,
            from_sqlite_timestamp(created_at),
            None,
        )?;
    }
    transaction.execute_batch("DROP TABLE legacy_messages; DROP TABLE legacy_session;")?;
    transaction.pragma_update(None, "application_id", APPLICATION_ID)?;
    transaction.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    transaction.commit()?;
    Ok(())
}

fn validate_database(connection: &Connection) -> Result<(), SessionError> {
    let check: String = connection.pragma_query_value(None, "quick_check", |row| row.get(0))?;
    if check != "ok" {
        return Err(SessionError::Corrupt(check));
    }
    load_metadata(connection)?;
    Ok(())
}

fn lock_session_file(path: &Path) -> Result<File, SessionError> {
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    fs2::FileExt::try_lock_exclusive(&file).map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock {
            SessionError::Locked(path.to_path_buf())
        } else {
            SessionError::Io(error)
        }
    })?;
    Ok(file)
}

fn is_session_locked(path: &Path) -> bool {
    let Ok(file) = OpenOptions::new().read(true).write(true).open(path) else {
        return false;
    };
    match fs2::FileExt::try_lock_exclusive(&file) {
        Ok(()) => {
            let _ = fs2::FileExt::unlock(&file);
            false
        }
        Err(error) => error.kind() == std::io::ErrorKind::WouldBlock,
    }
}

fn sanitize_title(title: &str) -> String {
    let title = title
        .lines()
        .next()
        .unwrap_or(DEFAULT_TITLE)
        .trim()
        .trim_matches(|character| matches!(character, '\'' | '"' | '`'))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TITLE_CHARS)
        .collect::<String>();
    let title = title.trim().trim_end_matches(['.', ',', ':', ';', '!', '?']).trim();
    if title.is_empty() {
        DEFAULT_TITLE.into()
    } else {
        title.into()
    }
}

fn session_slug(title: &str) -> String {
    let slug = slugify(title, MAX_SLUG_CHARS);
    if slug.is_empty() {
        "session".into()
    } else {
        slug
    }
}

fn project_directory_name(project_path: &Path) -> String {
    let slug = slugify(&project_path.to_string_lossy(), MAX_PROJECT_SLUG_CHARS);
    let slug = if slug.is_empty() { "project" } else { &slug };
    let hash = fnv1a(project_path.to_string_lossy().as_bytes());
    format!("--{slug}--{hash:016x}")
}

fn slugify(value: &str, max_chars: usize) -> String {
    let mut slug = String::new();
    let mut separator_pending = false;
    for character in value.chars() {
        if character.is_ascii_alphanumeric() {
            if separator_pending && !slug.is_empty() && slug.len() < max_chars {
                slug.push('-');
            }
            separator_pending = false;
            if slug.len() >= max_chars {
                break;
            }
            slug.push(character.to_ascii_lowercase());
        } else if !slug.is_empty() {
            separator_pending = true;
        }
    }
    slug.trim_end_matches('-').to_string()
}

fn canonical_path(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map(|cwd| cwd.join(path))
                .unwrap_or_else(|_| path.to_path_buf())
        }
    })
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn new_id() -> String {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let counter = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
    let process = u64::from(std::process::id());
    format!("{:016x}", timestamp ^ process.rotate_left(17) ^ counter)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn sqlite_timestamp(timestamp: u64) -> i64 {
    timestamp.min(i64::MAX as u64) as i64
}

fn from_sqlite_timestamp(timestamp: i64) -> u64 {
    timestamp.max(0) as u64
}

fn message_timestamp(message: &AgentMessage) -> u64 {
    match message {
        AgentMessage::User(message) => message.timestamp,
        AgentMessage::Assistant(message) => message.timestamp,
        AgentMessage::ToolResult(message) => message.timestamp,
    }
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::fs::DirBuilder;
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

    if !path.exists() {
        DirBuilder::new().mode(0o700).recursive(true).create(path)?;
    }
    let mut permissions = std::fs::metadata(path)?.permissions();
    if permissions.mode() & 0o077 != 0 {
        permissions.set_mode(0o700);
        std::fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map(drop)
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> std::io::Result<()> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(path)
        .map(drop)
}

#[cfg(test)]
mod tests {
    use super::{
        build_active_messages, build_display_messages, project_directory_name, session_summaries,
        CompactionRecord, SessionEntry, SessionError, SessionStore,
    };
    use crate::backend::agent::{AgentMessage, UserMessage};
    use std::path::{Path, PathBuf};

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "raid-session-test-{}-{}",
                std::process::id(),
                super::new_id()
            ));
            std::fs::create_dir_all(&path).expect("test directory");
            Self(path)
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn create(root: &Path, project: &Path) -> SessionStore {
        SessionStore::create_in(
            root,
            project,
            "system",
            "provider",
            "model",
            "openai-compatible",
        )
            .expect("session store")
    }

    #[test]
    fn each_session_gets_its_own_database_file() {
        let root = TestDir::new();
        let project = root.0.join("project");
        std::fs::create_dir_all(&project).expect("project");
        let first = create(&root.0, &project);
        let second = create(&root.0, &project);
        assert_ne!(first.path(), second.path());
        assert_eq!(first.path().parent(), second.path().parent());
    }

    #[test]
    fn different_projects_use_different_directories() {
        let root = TestDir::new();
        let first_project = root.0.join("first");
        let second_project = root.0.join("second");
        std::fs::create_dir_all(&first_project).expect("first project");
        std::fs::create_dir_all(&second_project).expect("second project");
        let first = create(&root.0, &first_project);
        let second = create(&root.0, &second_project);
        assert_ne!(first.path().parent(), second.path().parent());
    }

    #[test]
    fn messages_round_trip_on_the_active_path() {
        let root = TestDir::new();
        let project = root.0.join("project");
        std::fs::create_dir_all(&project).expect("project");
        let mut store = create(&root.0, &project);
        let first = AgentMessage::User(UserMessage::new("one"));
        let second = AgentMessage::User(UserMessage::new("two"));
        store.append_message(&first).expect("first message");
        store.append_message(&second).expect("second message");
        assert_eq!(store.snapshot().expect("snapshot").active_messages, [first, second]);
    }

    #[test]
    fn selecting_an_earlier_leaf_creates_an_independent_branch() {
        let root = TestDir::new();
        let project = root.0.join("project");
        std::fs::create_dir_all(&project).expect("project");
        let mut store = create(&root.0, &project);
        let first = AgentMessage::User(UserMessage::new("shared root"));
        let abandoned = AgentMessage::User(UserMessage::new("abandoned branch"));
        let replacement = AgentMessage::User(UserMessage::new("replacement branch"));
        let first_id = store.append_message(&first).expect("first message");
        store.append_message(&abandoned).expect("abandoned message");
        store.set_leaf(Some(&first_id)).expect("select root leaf");
        store.append_message(&replacement).expect("replacement message");

        assert_eq!(
            store.snapshot().expect("snapshot").active_messages,
            [first, replacement]
        );
    }

    #[test]
    fn model_changes_update_resume_metadata() {
        let root = TestDir::new();
        let project = root.0.join("project");
        std::fs::create_dir_all(&project).expect("project");
        let mut store = create(&root.0, &project);
        store
            .record_model_change("next-provider", "next-model", "anthropic-messages")
            .expect("model change");
        let metadata = store.metadata().expect("metadata");
        assert_eq!(metadata.current_provider, "next-provider");
        assert_eq!(metadata.current_model, "next-model");
        assert_eq!(metadata.current_api, "anthropic-messages");
    }

    #[test]
    fn version_two_databases_gain_protocol_metadata_on_open() {
        let root = TestDir::new();
        let project = root.0.join("project");
        std::fs::create_dir_all(&project).expect("project");
        let path = root.0.join("legacy-v2.db");
        let connection = rusqlite::Connection::open(&path).expect("legacy database");
        connection
            .execute_batch(
                "CREATE TABLE entries (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                    id TEXT NOT NULL UNIQUE,
                    parent_id TEXT REFERENCES entries(id),
                    kind TEXT NOT NULL,
                    role TEXT,
                    payload_json TEXT NOT NULL,
                    created_at INTEGER NOT NULL
                 );
                 CREATE TABLE session (
                    singleton INTEGER PRIMARY KEY,
                    id TEXT NOT NULL UNIQUE,
                    title TEXT NOT NULL,
                    title_generated INTEGER NOT NULL,
                    project_path TEXT NOT NULL,
                    canonical_project_path TEXT NOT NULL,
                    system_prompt_snapshot TEXT NOT NULL,
                    initial_provider TEXT NOT NULL,
                    initial_model TEXT NOT NULL,
                    current_provider TEXT NOT NULL,
                    current_model TEXT NOT NULL,
                    parent_session TEXT,
                    last_entry_id TEXT,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                 );
                 PRAGMA user_version = 2;",
            )
            .expect("version two schema");
        connection
            .execute(
                "INSERT INTO session VALUES (1, 'legacy', 'Legacy', 0, ?1, ?1, 'prompt',
                 'provider', 'model', 'provider', 'model', NULL, NULL, 1, 1)",
                [project.to_string_lossy().as_ref()],
            )
            .expect("legacy metadata");
        drop(connection);

        let store = SessionStore::open(&path).expect("migrated store");
        let metadata = store.metadata().expect("migrated metadata");
        assert_eq!(metadata.current_api, "openai-compatible");
        let version: i64 = store
            .connection_ref()
            .expect("connection")
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("schema version");
        assert_eq!(version, super::SCHEMA_VERSION);
    }

    #[test]
    fn version_one_messages_migrate_into_the_entry_tree() {
        let root = TestDir::new();
        let project = root.0.join("project");
        std::fs::create_dir_all(&project).expect("project");
        let path = root.0.join("legacy-v1.db");
        let connection = rusqlite::Connection::open(&path).expect("legacy database");
        connection
            .execute_batch(
                "CREATE TABLE session (
                    id TEXT NOT NULL, title TEXT NOT NULL, project_path TEXT NOT NULL,
                    system_prompt TEXT NOT NULL, provider TEXT NOT NULL, model TEXT NOT NULL,
                    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL
                 );
                 CREATE TABLE messages (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT, role TEXT NOT NULL,
                    payload_json TEXT NOT NULL, created_at INTEGER NOT NULL
                 );
                 PRAGMA user_version = 1;",
            )
            .expect("version one schema");
        connection
            .execute(
                "INSERT INTO session VALUES ('legacy', 'Legacy', ?1, 'prompt', 'provider',
                 'model', 1, 1)",
                [project.to_string_lossy().as_ref()],
            )
            .expect("legacy metadata");
        let message = AgentMessage::User(UserMessage::new("migrated message"));
        connection
            .execute(
                "INSERT INTO messages (role, payload_json, created_at) VALUES ('user', ?1, 1)",
                [serde_json::to_string(&message).expect("message json")],
            )
            .expect("legacy message");
        drop(connection);

        let store = SessionStore::open(&path).expect("migrated store");
        assert_eq!(
            store.snapshot().expect("snapshot").active_messages,
            [message]
        );
        assert_eq!(
            store.metadata().expect("metadata").current_api,
            "openai-compatible"
        );
    }

    #[test]
    fn title_update_renames_the_database_and_updates_metadata() {
        let root = TestDir::new();
        let project = root.0.join("project");
        std::fs::create_dir_all(&project).expect("project");
        let mut store = create(&root.0, &project);
        let old_path = store.path().to_path_buf();
        store.set_title("Fix session resume", true).expect("set title");
        assert!(!old_path.exists());
        assert!(store.path().exists());
        assert!(store.path().file_name().unwrap().to_string_lossy().starts_with("fix-session-resume--"));
        assert_eq!(store.metadata().expect("metadata").title, "Fix session resume");
    }

    #[test]
    fn project_paths_are_readable_and_collision_resistant() {
        let first = project_directory_name(Path::new("/tmp/one/project"));
        let second = project_directory_name(Path::new("/tmp/two/project"));
        assert!(first.starts_with("--tmp-one-project--"));
        assert_ne!(first, second);
        assert!(first.len() <= 116);
    }

    #[test]
    fn opening_an_active_session_is_rejected() {
        let root = TestDir::new();
        let project = root.0.join("project");
        std::fs::create_dir_all(&project).expect("project");
        let store = create(&root.0, &project);
        let error = SessionStore::open(store.path()).err().expect("locked error");
        assert!(matches!(error, SessionError::Locked(_)));
    }

    #[test]
    fn summaries_are_sorted_by_activity() {
        let root = TestDir::new();
        let project = root.0.join("project");
        std::fs::create_dir_all(&project).expect("project");
        let mut first = create(&root.0, &project);
        first.set_title("First", true).expect("first title");
        let mut second = create(&root.0, &project);
        second.set_title("Second", true).expect("second title");
        second
            .append_message(&AgentMessage::User(UserMessage::new("later")))
            .expect("later message");
        let summaries = session_summaries(&root.0, &project).expect("summaries");
        assert_eq!(summaries[0].title, "Second");
    }

    #[test]
    fn compaction_replaces_earlier_context() {
        let first = AgentMessage::User(UserMessage::new("old"));
        let kept = AgentMessage::User(UserMessage::new("kept"));
        let first_entry = SessionEntry {
            sequence: 1,
            id: "one".into(),
            parent_id: None,
            kind: "message".into(),
            role: Some("user".into()),
            payload: serde_json::to_value(first).expect("first payload"),
            created_at: 1,
        };
        let kept_entry = SessionEntry {
            sequence: 2,
            id: "two".into(),
            parent_id: Some("one".into()),
            kind: "message".into(),
            role: Some("user".into()),
            payload: serde_json::to_value(&kept).expect("kept payload"),
            created_at: 2,
        };
        let compact = SessionEntry {
            sequence: 3,
            id: "three".into(),
            parent_id: Some("two".into()),
            kind: "compaction".into(),
            role: None,
            payload: serde_json::to_value(CompactionRecord {
                summary: "summary".into(),
                first_kept_entry_id: Some("two".into()),
                tokens_before: 100,
                retained_tail: Vec::new(),
                details: None,
            })
            .expect("compaction payload"),
            created_at: 3,
        };
        let messages = build_active_messages(&[first_entry, kept_entry, compact], Some("three"))
            .expect("active messages");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1], kept);

        let first = AgentMessage::User(UserMessage::new("old"));
        let kept = AgentMessage::User(UserMessage::new("kept"));
        let entries = vec![
            SessionEntry {
                sequence: 1,
                id: "one".into(),
                parent_id: None,
                kind: "message".into(),
                role: Some("user".into()),
                payload: serde_json::to_value(&first).expect("first payload"),
                created_at: 1,
            },
            SessionEntry {
                sequence: 2,
                id: "two".into(),
                parent_id: Some("one".into()),
                kind: "message".into(),
                role: Some("user".into()),
                payload: serde_json::to_value(&kept).expect("kept payload"),
                created_at: 2,
            },
            SessionEntry {
                sequence: 3,
                id: "three".into(),
                parent_id: Some("two".into()),
                kind: "compaction".into(),
                role: None,
                payload: serde_json::to_value(CompactionRecord {
                    summary: "summary".into(),
                    first_kept_entry_id: Some("two".into()),
                    tokens_before: 100,
                    retained_tail: Vec::new(),
                    details: None,
                })
                .expect("compaction payload"),
                created_at: 3,
            },
        ];
        let display = build_display_messages(&entries, Some("three")).expect("display messages");
        assert_eq!(display, [first, kept]);
    }

    #[test]
    fn compaction_references_the_retained_tail_without_duplicating_it() {
        let root = TestDir::new();
        let project = root.0.join("project");
        std::fs::create_dir_all(&project).expect("project");
        let mut store = create(&root.0, &project);
        let old = AgentMessage::User(UserMessage::new("old"));
        let kept = AgentMessage::User(UserMessage::new("kept"));
        store.append_message(&old).expect("old message");
        let kept_id = store.append_message(&kept).expect("kept message");
        store
            .append_compaction(&CompactionRecord {
                summary: "checkpoint".into(),
                first_kept_entry_id: None,
                tokens_before: 100,
                retained_tail: vec![kept.clone()],
                details: None,
            })
            .expect("compaction");

        let connection = rusqlite::Connection::open(store.path()).expect("connection");
        let payload: String = connection
            .query_row(
                "SELECT payload_json FROM entries WHERE kind = 'compaction'",
                [],
                |row| row.get(0),
            )
            .expect("compaction payload");
        let record: CompactionRecord = serde_json::from_str(&payload).expect("record");
        assert_eq!(record.first_kept_entry_id.as_deref(), Some(kept_id.as_str()));
        assert!(record.retained_tail.is_empty());

        let snapshot = store.snapshot().expect("snapshot");
        assert_eq!(snapshot.active_messages.len(), 2);
        assert_eq!(snapshot.active_messages[1], kept);
        assert_eq!(snapshot.display_messages, [old, kept]);
    }

    #[cfg(unix)]
    #[test]
    fn database_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let root = TestDir::new();
        let project = root.0.join("project");
        std::fs::create_dir_all(&project).expect("project");
        let store = create(&root.0, &project);
        let mode = std::fs::metadata(store.path())
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }
}
