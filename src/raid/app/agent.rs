use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use tokio::runtime::Handle;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::backend::agent::{
    AgentContext, AgentEvent, AgentLoopConfig, AgentLoopHandle, AgentMessage, AssistantContent,
    AssistantMessage, AssistantMessageEvent, LlmContext, LlmMessage, Model, StopReason, StreamFn,
    StreamOptions, UserMessage, agent_loop, get_default_stream_fn, set_default_stream_fn,
};
use crate::backend::compaction::{CompactionOutcome, CompactionRequest, compact, should_compact};
use crate::backend::opencode::types::ReasoningVariant;
use crate::backend::session::{SessionStore, SessionSummary, most_recent_session};
use crate::backend::tools::{ToolEnvironment, default_tools};
use crate::config::{RaidSettings, load_provider_catalog_from_disk, resolve_api_key, sessions_dir};
use crate::frontend::chat::{Role, ViewportState};
use crate::frontend::tools::ToolStatus;
use serde_json::Value;

pub struct AgentSession {
    handle: Option<AgentLoopHandle>,
    context: AgentContext,
    config: AgentLoopConfig,
    model_limits: (u64, u64),
    cancel: CancellationToken,
    tool_indices: HashMap<String, usize>,
    assistant_index: Option<usize>,
    runtime: Handle,
    stream_fn: Option<StreamFn>,
    project_path: PathBuf,
    session_root: Option<PathBuf>,
    store: Option<SessionStore>,
    persisted_messages: HashSet<String>,
    title_task: Option<JoinHandle<Result<String, String>>>,
    compaction_task: Option<JoinHandle<Result<CompactionOutcome, String>>>,
    manual_compaction: bool,
    pending_submit: Option<String>,
    activity_header: String,
    activity_visible: bool,
    reasoning_buffer: String,
}

impl AgentSession {
    pub fn new(runtime: Handle) -> Self {
        let settings = RaidSettings::load();
        let provider_id = settings.provider_id();
        let model_id = settings.model_id().to_string();
        let project_path = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let model = Model {
            id: model_id.clone(),
            name: model_id,
            api: settings.api().into(),
            provider: provider_id.to_string(),
        };
        let model_limits = current_model_limits(&model);
        let mut config = AgentLoopConfig::new(
            model,
            Arc::new(crate::backend::opencode::convert_agent_messages),
        );
        config.api_key = resolve_api_key(provider_id);
        let tool_env = Arc::new(ToolEnvironment::with_cwd(project_path.clone()));
        Self {
            handle: None,
            context: AgentContext {
                system_prompt: build_system_prompt(&project_path),
                messages: Vec::new(),
                tools: default_tools(tool_env),
            },
            config,
            model_limits,
            cancel: CancellationToken::new(),
            tool_indices: HashMap::new(),
            assistant_index: None,
            runtime,
            stream_fn: None,
            project_path,
            session_root: if cfg!(test) {
                None
            } else {
                Some(sessions_dir())
            },
            store: None,
            persisted_messages: HashSet::new(),
            title_task: None,
            compaction_task: None,
            manual_compaction: false,
            pending_submit: None,
            activity_header: String::from("Working"),
            activity_visible: false,
            reasoning_buffer: String::new(),
        }
    }

    pub fn reload_credentials(&mut self) {
        let previous_provider = self.config.model.provider.clone();
        let previous_model = self.config.model.id.clone();
        let settings = RaidSettings::load();
        let provider_id = settings.provider_id();
        let model_id = settings.model_id().to_string();
        self.config.model.id = model_id.clone();
        self.config.model.name = model_id;
        self.config.model.api = settings.api().into();
        self.config.model.provider = provider_id.to_string();
        self.config.api_key = resolve_api_key(provider_id);
        self.model_limits = current_model_limits(&self.config.model);
        if (previous_provider != self.config.model.provider
            || previous_model != self.config.model.id)
            && let Some(store) = &mut self.store
            && let Err(error) = store.record_model_change(
                &self.config.model.provider,
                &self.config.model.id,
                &self.config.model.api,
            )
        {
            tracing::warn!(%error, path = %store.path().display(), "session model change write failed");
        }
    }

    #[cfg(test)]
    pub fn with_stream_fn(mut self, stream_fn: StreamFn) -> Self {
        self.stream_fn = Some(stream_fn);
        self
    }

    pub fn is_running(&self) -> bool {
        self.handle.is_some() || self.compaction_task.is_some()
    }

    pub fn activity_header(&self) -> Option<&str> {
        (self.is_running() && self.activity_visible).then_some(self.activity_header.as_str())
    }

    pub fn interrupt(&mut self) -> bool {
        if !self.is_running() {
            return false;
        }
        self.cancel.cancel();
        if self.compaction_task.is_some() {
            self.pending_submit = None;
        }
        true
    }

    pub fn submit(&mut self, message: String) {
        if self.is_running() {
            return;
        }
        if self.should_compact_before(&message) {
            self.pending_submit = Some(message);
            self.spawn_compaction(None, false);
            return;
        }
        self.submit_now(message);
    }

    fn submit_now(&mut self, message: String) {
        self.cancel = CancellationToken::new();
        self.tool_indices.clear();
        self.assistant_index = None;
        self.activity_header = String::from("Working");
        self.activity_visible = true;
        self.reasoning_buffer.clear();
        let prompt = AgentMessage::User(UserMessage::new(message.clone()));
        self.start_session(&message);
        self.persist_message(&prompt);
        let prior_context = self.context.clone();
        self.context.messages.push(prompt.clone());
        let handle = agent_loop(
            vec![prompt],
            prior_context,
            self.config.clone(),
            self.cancel.clone(),
            self.stream_fn.clone(),
        );
        let AgentLoopHandle { events, result } = handle;
        let task = self.runtime.spawn(result);
        self.handle = Some(AgentLoopHandle {
            events,
            result: Box::pin(async move { task.await.expect("agent loop driver task") }),
        });
    }

    pub fn poll(&mut self, chat: &mut ViewportState) {
        self.poll_title();
        self.poll_compaction(chat);
        let Some(mut handle) = self.handle.take() else {
            return;
        };

        loop {
            match handle.events.try_recv() {
                Ok(event) => self.apply_event(chat, event),
                Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                    self.handle = Some(handle);
                    return;
                }
                Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
                    let _ = self.runtime.block_on(handle.result);
                    self.handle = None;
                    self.activity_visible = false;
                    return;
                }
            }
        }
    }

    fn apply_event(&mut self, chat: &mut ViewportState, event: AgentEvent) {
        match event {
            AgentEvent::MessageStart { message } => self.on_message_start(chat, message),
            AgentEvent::MessageUpdate {
                message,
                assistant_message_event,
            } => {
                self.on_assistant_stream_event(&assistant_message_event);
                self.on_message_update(chat, message);
            }
            AgentEvent::MessageEnd { message } => self.on_message_end(chat, message),
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                self.activity_header = String::from("Working");
                self.activity_visible = true;
                self.reasoning_buffer.clear();
                let detail = format_tool_args(&tool_name, &args);
                let index = chat.start_tool(tool_name, detail);
                self.tool_indices.insert(tool_call_id, index);
            }
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
                is_error,
            } => {
                self.activity_header = String::from("Working");
                self.activity_visible = true;
                if let Some(index) = self.tool_indices.get(&tool_call_id).copied() {
                    let summary = result
                        .content
                        .iter()
                        .filter_map(|part| part.as_text())
                        .collect::<Vec<_>>()
                        .join("");
                    let summary = if summary.is_empty() {
                        format!("{tool_name} finished")
                    } else {
                        summary
                    };
                    chat.finish_tool(
                        index,
                        if is_error {
                            ToolStatus::Failed
                        } else {
                            ToolStatus::Success
                        },
                        summary,
                    );
                }
            }
            AgentEvent::AgentEnd { messages } => {
                for message in messages {
                    if message.role() != "user" {
                        self.context.messages.push(message);
                    }
                }
            }
            AgentEvent::TurnEnd {
                message,
                tool_results,
            } => {
                let _ = (message, tool_results);
            }
            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                partial_result,
                ..
            } => {
                if let Some(index) = self.tool_indices.get(&tool_call_id).copied() {
                    let summary = partial_result
                        .content
                        .iter()
                        .filter_map(|part| part.as_text())
                        .collect::<Vec<_>>()
                        .join("");
                    if !summary.is_empty() {
                        chat.update_tool(index, summary);
                    }
                }
            }
            _ => {}
        }
    }

    fn on_message_start(&mut self, chat: &mut ViewportState, message: AgentMessage) {
        if let AgentMessage::Assistant(assistant) = message {
            let text = assistant_text(&assistant);
            self.assistant_index = (!text.is_empty()).then(|| chat.append_assistant(text));
        }
    }

    fn on_assistant_stream_event(&mut self, event: &AssistantMessageEvent) {
        match event {
            AssistantMessageEvent::ThinkingStart { .. } => {
                self.activity_visible = true;
                self.reasoning_buffer.clear();
            }
            AssistantMessageEvent::ThinkingDelta { delta, .. } => {
                self.activity_visible = true;
                self.reasoning_buffer.push_str(delta);
                if let Some(header) = extract_first_bold(&self.reasoning_buffer) {
                    self.activity_header = header;
                }
            }
            AssistantMessageEvent::ThinkingEnd { .. } => {
                self.activity_visible = true;
            }
            AssistantMessageEvent::ToolcallStart { .. }
            | AssistantMessageEvent::ToolcallDelta { .. }
            | AssistantMessageEvent::ToolcallEnd { .. } => {
                self.activity_header = String::from("Working");
                self.activity_visible = true;
            }
            _ => {}
        }
    }

    fn on_message_update(&mut self, chat: &mut ViewportState, message: AgentMessage) {
        if let AgentMessage::Assistant(assistant) = message {
            let text = assistant_text(&assistant);
            if text.is_empty() {
                return;
            }
            if let Some(index) = self.assistant_index {
                chat.update_assistant(index, text);
            } else {
                self.assistant_index = Some(chat.append_assistant(text));
            }
        }
    }

    fn on_message_end(&mut self, chat: &mut ViewportState, message: AgentMessage) {
        self.persist_message(&message);
        if let AgentMessage::Assistant(assistant) = message {
            let text = assistant_text(&assistant);
            if !text.is_empty() {
                if let Some(index) = self.assistant_index {
                    chat.update_assistant(index, text);
                } else {
                    chat.append_assistant(text);
                }
            }
            self.assistant_index = None;
        }
    }

    fn start_session(&mut self, first_message: &str) {
        if self.store.is_some() {
            return;
        }
        let Some(session_root) = &self.session_root else {
            return;
        };

        match SessionStore::create_in(
            session_root,
            &self.project_path,
            &self.context.system_prompt,
            &self.config.model.provider,
            &self.config.model.id,
            &self.config.model.api,
        ) {
            Ok(store) => {
                tracing::info!(path = %store.path().display(), "created session database");
                self.store = Some(store);
                self.spawn_title_generation(first_message);
            }
            Err(error) => tracing::warn!(%error, "session database creation failed"),
        }
    }

    fn persist_message(&mut self, message: &AgentMessage) {
        let key = message_key(message);
        if self.persisted_messages.contains(&key) {
            return;
        }
        let Some(store) = &mut self.store else {
            return;
        };
        match store.append_message(message) {
            Ok(_) => {
                self.persisted_messages.insert(key);
            }
            Err(error) => {
                tracing::warn!(%error, path = %store.path().display(), "session write failed");
            }
        }
    }

    fn spawn_title_generation(&mut self, first_message: &str) {
        if self.title_task.is_some() {
            return;
        }
        let stream_fn = self.stream_fn.clone().unwrap_or_else(get_default_stream_fn);
        let settings = RaidSettings::load();
        let model = text_generation_model(&settings);
        let api_key = resolve_api_key(&model.provider);
        let message = first_message.chars().take(8_000).collect::<String>();
        self.title_task = Some(
            self.runtime
                .spawn(generate_session_title(stream_fn, model, api_key, message)),
        );
    }

    pub fn retry_session_title(&mut self) {
        let first_message = {
            let Some(store) = &self.store else {
                return;
            };
            match store.current_title_is_replaceable() {
                Ok(true) => {}
                Ok(false) => return,
                Err(error) => {
                    tracing::warn!(%error, path = %store.path().display(), "session title check failed");
                    return;
                }
            }
            match store.snapshot() {
                Ok(snapshot) => first_user_text(&snapshot.active_messages),
                Err(error) => {
                    tracing::warn!(%error, path = %store.path().display(), "session title retry read failed");
                    return;
                }
            }
        };
        let Some(first_message) = first_message else {
            return;
        };
        if let Some(task) = self.title_task.take() {
            task.abort();
        }
        self.spawn_title_generation(&first_message);
    }

    fn poll_title(&mut self) {
        let Some(task) = self.title_task.as_ref() else {
            return;
        };
        if !task.is_finished() {
            return;
        }
        let task = self.title_task.take().expect("checked title task");
        let title = match self.runtime.block_on(task) {
            Ok(Ok(title)) => title,
            Ok(Err(error)) => {
                tracing::warn!(%error, "session title generation failed");
                return;
            }
            Err(error) => {
                tracing::warn!(%error, "session title task failed");
                return;
            }
        };
        let Some(store) = &mut self.store else {
            return;
        };
        match store.current_title_is_replaceable() {
            Ok(true) => {
                if let Err(error) = store.set_title(&title, true) {
                    tracing::warn!(%error, path = %store.path().display(), "session title update failed");
                }
            }
            Ok(false) => {}
            Err(error) => {
                tracing::warn!(%error, path = %store.path().display(), "session title check failed");
            }
        }
    }

    fn should_compact_before(&self, next_message: &str) -> bool {
        let (context_limit, _) = self.model_limits;
        should_compact(&self.context.messages, next_message, context_limit)
    }

    fn spawn_compaction(&mut self, custom_instructions: Option<String>, manual: bool) {
        let messages = self.context.messages.clone();
        let stream_fn = self.stream_fn.clone().unwrap_or_else(get_default_stream_fn);
        let model = self.config.model.clone();
        let api_key = self.config.api_key.clone();
        let (context_limit, output_limit) = self.model_limits;
        self.cancel = CancellationToken::new();
        self.activity_header = String::from("Working");
        self.activity_visible = true;
        self.reasoning_buffer.clear();
        self.manual_compaction = manual;
        self.compaction_task = Some(self.runtime.spawn(compact(CompactionRequest {
            stream_fn,
            model,
            api_key,
            messages,
            context_limit,
            output_limit,
            custom_instructions,
            cancel: self.cancel.clone(),
        })));
    }

    pub fn compact(&mut self, custom_instructions: String) -> Result<(), String> {
        if self.is_running() {
            return Err("Wait for the current response before compacting this session.".into());
        }
        if self.context.messages.is_empty() {
            return Err("There is no conversation to compact yet.".into());
        }
        let custom_instructions = (!custom_instructions.trim().is_empty())
            .then(|| custom_instructions.trim().to_string());
        self.spawn_compaction(custom_instructions, true);
        Ok(())
    }

    fn poll_compaction(&mut self, chat: &mut ViewportState) {
        let Some(task) = self.compaction_task.as_ref() else {
            return;
        };
        if !task.is_finished() {
            return;
        }
        let task = self
            .compaction_task
            .take()
            .expect("checked compaction task");
        let manual = std::mem::take(&mut self.manual_compaction);
        match self.runtime.block_on(task) {
            Ok(Ok(outcome)) => {
                let persisted = if let Some(store) = &mut self.store {
                    match store.append_compaction(&outcome.record) {
                        Ok(_) => true,
                        Err(error) => {
                            tracing::warn!(%error, path = %store.path().display(), "session compaction write failed");
                            chat.push(
                                Role::Assistant,
                                format!("Could not save the compacted session context: {error}"),
                            );
                            false
                        }
                    }
                } else {
                    true
                };
                if !persisted {
                    if let Some(message) = self.pending_submit.take() {
                        self.submit_now(message);
                    }
                    return;
                }
                self.context.messages = outcome.messages;
                if manual {
                    chat.push(
                        Role::Assistant,
                        format!(
                            "Compacted {} tokens to about {}. Recent messages were kept unchanged.",
                            outcome.record.tokens_before, outcome.estimated_tokens_after
                        ),
                    );
                }
            }
            Ok(Err(error)) => {
                tracing::warn!(%error, "session compaction failed");
                chat.push(
                    Role::Assistant,
                    format!("Could not compact the older session context: {error}"),
                );
            }
            Err(error) => {
                tracing::warn!(%error, "session compaction task failed");
            }
        }
        if let Some(message) = self.pending_submit.take() {
            self.submit_now(message);
        }
    }

    pub fn new_session(&mut self, chat: &mut ViewportState) -> Result<(), String> {
        if self.is_running() {
            return Err("Wait for the current response before starting a new session.".into());
        }
        if let Some(task) = self.title_task.take() {
            task.abort();
        }
        if let Some(task) = self.compaction_task.take() {
            task.abort();
        }
        self.pending_submit = None;
        self.manual_compaction = false;
        self.activity_visible = false;
        self.reasoning_buffer.clear();
        self.store = None;
        self.context.messages.clear();
        self.persisted_messages.clear();
        self.tool_indices.clear();
        self.assistant_index = None;
        chat.clear();
        self.reload_credentials();
        Ok(())
    }

    pub fn open_session(
        &mut self,
        path: impl AsRef<std::path::Path>,
        chat: &mut ViewportState,
    ) -> Result<(), String> {
        if self.is_running() {
            return Err("Wait for the current response before resuming another session.".into());
        }
        let store = SessionStore::open(path).map_err(|error| error.to_string())?;
        let snapshot = store.snapshot().map_err(|error| error.to_string())?;
        if !snapshot.metadata.canonical_project_path.exists() {
            return Err(format!(
                "The session project no longer exists: {}",
                snapshot.metadata.canonical_project_path.display()
            ));
        }
        let current_project =
            std::fs::canonicalize(&self.project_path).unwrap_or_else(|_| self.project_path.clone());
        if snapshot.metadata.canonical_project_path != current_project {
            return Err(format!(
                "This session belongs to {}. Start Raid there to resume it.",
                snapshot.metadata.canonical_project_path.display()
            ));
        }

        if let Some(task) = self.title_task.take() {
            task.abort();
        }
        if let Some(task) = self.compaction_task.take() {
            task.abort();
        }
        self.pending_submit = None;
        self.manual_compaction = false;
        self.activity_visible = false;
        self.reasoning_buffer.clear();
        self.config.model.provider = snapshot.metadata.current_provider.clone();
        self.config.model.id = snapshot.metadata.current_model.clone();
        self.config.model.name = snapshot.metadata.current_model.clone();
        self.config.model.api = snapshot.metadata.current_api.clone();
        self.config.api_key = resolve_api_key(&snapshot.metadata.current_provider);
        self.model_limits = current_model_limits(&self.config.model);
        self.context.messages = snapshot.active_messages.clone();
        self.persisted_messages = snapshot.active_messages.iter().map(message_key).collect();
        hydrate_chat(chat, &snapshot.display_messages);
        self.store = Some(store);
        self.retry_session_title();
        Ok(())
    }

    pub fn open_most_recent(&mut self, chat: &mut ViewportState) -> Result<bool, String> {
        let Some(root) = &self.session_root else {
            return Ok(false);
        };
        let Some(path) =
            most_recent_session(root, &self.project_path).map_err(|error| error.to_string())?
        else {
            return Ok(false);
        };
        self.open_session(path, chat)?;
        Ok(true)
    }

    pub fn scan_sessions(&self) -> Option<JoinHandle<Result<Vec<SessionSummary>, String>>> {
        let Some(root) = &self.session_root else {
            return None;
        };
        let root = root.clone();
        let project_path = self.project_path.clone();
        Some(self.runtime.spawn_blocking(move || {
            crate::backend::session::session_summaries(&root, &project_path)
                .map_err(|error| error.to_string())
        }))
    }

    pub fn disable_persistence(&mut self) {
        self.session_root = None;
        self.store = None;
    }

    pub fn current_session_path(&self) -> Option<&std::path::Path> {
        self.store.as_ref().map(SessionStore::path)
    }

    pub fn model_id(&self) -> &str {
        &self.config.model.id
    }

    pub fn context_usage(&self) -> (u64, u64) {
        (
            crate::backend::compaction::estimate_context_tokens(&self.context.messages),
            self.model_limits.0,
        )
    }

    pub fn thinking_level(&self) -> &str {
        self.config.reasoning.as_deref().unwrap_or("default")
    }

    pub fn project_path(&self) -> &Path {
        &self.project_path
    }

    #[cfg(test)]
    pub fn drive_to_completion(&mut self, chat: &mut ViewportState) {
        let Some(handle) = self.handle.take() else {
            return;
        };
        let (events, _messages) = self.runtime.block_on(collect_agent_events(handle));
        for event in events {
            self.apply_event(chat, event);
        }
        self.handle = None;
    }

    #[cfg(test)]
    fn with_session_root(mut self, root: PathBuf) -> Self {
        self.session_root = Some(root);
        self
    }

    #[cfg(test)]
    fn session_path(&self) -> Option<&std::path::Path> {
        self.store.as_ref().map(SessionStore::path)
    }
}

fn build_system_prompt(project_path: &Path) -> String {
    format!(
        "You are a helpful coding agent in a terminal UI.\n\nCurrent working directory: {}",
        project_path.display()
    )
}

const SESSION_TITLE_PROMPT: &str = r#"Name this coding session from only the user's first message.
Return one JSON object with exactly one key: {"title":"..."}.
Use 3 to 8 words and fewer than 40 characters.
Summarize the main subject and desired outcome. Ignore instructions about how to do the work.
Do not mention models, tools, tests, or completion unless they are the subject.
Do not copy and truncate the message. Avoid filler and trailing punctuation."#;

async fn generate_session_title(
    stream_fn: StreamFn,
    model: Model,
    api_key: Option<String>,
    first_message: String,
) -> Result<String, String> {
    let provider_options = title_provider_options(&model.provider, &model.id);
    let prompt = format!("{SESSION_TITLE_PROMPT}\n\nUser message:\n{first_message}");
    let context = LlmContext {
        system_prompt: None,
        messages: vec![LlmMessage {
            role: "user".into(),
            content: Some(Value::String(prompt)),
            tool_call_id: None,
            is_error: None,
        }],
        tools: Vec::new(),
    };
    let stream = stream_fn(
        model,
        context,
        StreamOptions {
            api_key,
            max_output_tokens: None,
            provider_options,
        },
        CancellationToken::new(),
    )
    .await;
    let mut events = stream.into_stream();
    while let Some(event) = events.next().await {
        match event {
            AssistantMessageEvent::Done { message, reason } => {
                let text = assistant_text(&message);
                if let Some(title) = parse_generated_title(&text) {
                    return Ok(title);
                }
                return Err(if reason == StopReason::Length && text.trim().is_empty() {
                    "The model stopped before returning title JSON.".into()
                } else {
                    format!("The model returned invalid title JSON with stop reason {reason:?}.")
                });
            }
            AssistantMessageEvent::Error { reason, error } => {
                return Err(error.error_message.unwrap_or_else(|| {
                    format!("The title request failed with stop reason {reason:?}.")
                }));
            }
            _ => {}
        }
    }
    Err("The title response stream ended before returning a title.".into())
}

fn text_generation_model(settings: &RaidSettings) -> Model {
    let model_id = settings.text_generation_model_id().to_string();
    Model {
        id: model_id.clone(),
        name: model_id,
        api: settings.text_generation_api().into(),
        provider: settings.text_generation_provider_id().to_string(),
    }
}

fn title_provider_options(provider_id: &str, model_id: &str) -> Option<Value> {
    let catalog = load_provider_catalog_from_disk(provider_id).ok()?;
    let model = catalog.models.iter().find(|model| model.id == model_id)?;
    if !model.reasoning {
        return None;
    }
    let variant = lowest_title_reasoning_variant(&model.reasoning_variants)?;
    serde_json::to_value(&variant.provider_options).ok()
}

fn lowest_title_reasoning_variant(variants: &[ReasoningVariant]) -> Option<&ReasoningVariant> {
    variants
        .iter()
        .min_by_key(|variant| title_reasoning_rank(&variant.id, &variant.label))
}

fn title_reasoning_rank(id: &str, label: &str) -> usize {
    let id = id.to_ascii_lowercase();
    let label = label.to_ascii_lowercase();
    if id == "toggle:disabled"
        || id == "effort:none"
        || matches!(label.as_str(), "off" | "none" | "disabled")
    {
        return 0;
    }
    match label.as_str() {
        "minimal" => 1,
        "low" => 2,
        "medium" => 3,
        "high" => 4,
        "xhigh" => 5,
        "max" => 6,
        "default" => usize::MAX,
        _ => 100,
    }
}

fn current_model_limits(model: &Model) -> (u64, u64) {
    load_provider_catalog_from_disk(&model.provider)
        .ok()
        .and_then(|catalog| {
            catalog
                .models
                .into_iter()
                .find(|candidate| candidate.id == model.id)
                .map(|candidate| (candidate.context_limit, candidate.output_limit))
        })
        .unwrap_or((128_000, 8_192))
}

fn first_user_text(messages: &[AgentMessage]) -> Option<String> {
    messages.iter().find_map(|message| match message {
        AgentMessage::User(user) => user.content.as_str().map(str::to_string),
        _ => None,
    })
}

fn parse_generated_title(raw: &str) -> Option<String> {
    let candidate = json_object_candidates(raw).find_map(|json_text| {
        let value = serde_json::from_str::<Value>(json_text).ok()?;
        let object = value.as_object()?;
        if object.len() != 1 {
            return None;
        }
        object.get("title")?.as_str().map(str::to_string)
    })?;
    let candidate = candidate
        .trim()
        .trim_matches(|character| matches!(character, '\'' | '"' | '`'))
        .split_whitespace()
        .take(8)
        .collect::<Vec<_>>()
        .join(" ");
    let candidate = candidate
        .chars()
        .take(40)
        .collect::<String>()
        .trim()
        .trim_end_matches(['.', ',', ':', ';', '!', '?'])
        .trim()
        .to_string();
    (!candidate.is_empty()).then_some(candidate)
}

fn json_object_candidates(raw: &str) -> impl Iterator<Item = &str> {
    raw.match_indices('{').filter_map(|(start, _)| {
        let object = &raw[start..];
        matching_json_object_end(object).map(|end| &object[..end])
    })
}

fn matching_json_object_end(input: &str) -> Option<usize> {
    let mut depth = 0_usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, character) in input.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => depth = depth.saturating_add(1),
            '}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index + character.len_utf8());
                }
            }
            _ => {}
        }
    }
    None
}

fn hydrate_chat(chat: &mut ViewportState, messages: &[AgentMessage]) {
    chat.clear();
    let mut tool_indices = HashMap::new();
    for message in messages {
        match message {
            AgentMessage::User(message) => {
                let text = value_text(&message.content);
                if !text.is_empty() {
                    chat.push(Role::User, text);
                }
            }
            AgentMessage::Assistant(message) => {
                let text = assistant_text(message);
                if !text.is_empty() {
                    chat.push(Role::Assistant, text);
                }
                for content in &message.content {
                    if let AssistantContent::ToolCall(call) = content {
                        let index = chat
                            .start_tool(&call.name, format_tool_args(&call.name, &call.arguments));
                        tool_indices.insert(call.id.clone(), index);
                    }
                }
            }
            AgentMessage::ToolResult(message) => {
                let summary = message
                    .content
                    .iter()
                    .filter_map(|part| part.as_text())
                    .collect::<Vec<_>>()
                    .join("");
                let index = tool_indices
                    .get(&message.tool_call_id)
                    .copied()
                    .unwrap_or_else(|| chat.start_tool(&message.tool_name, "{}"));
                chat.finish_tool(
                    index,
                    if message.is_error {
                        ToolStatus::Failed
                    } else {
                        ToolStatus::Success
                    },
                    if summary.is_empty() {
                        format!("{} finished", message.tool_name)
                    } else {
                        summary
                    },
                );
            }
        }
    }
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

fn message_key(message: &AgentMessage) -> String {
    serde_json::to_string(message).unwrap_or_else(|_| format!("{}:{message:?}", message.role()))
}

#[cfg(test)]
async fn collect_agent_events(mut handle: AgentLoopHandle) -> (Vec<AgentEvent>, Vec<AgentMessage>) {
    let mut events = Vec::new();
    let mut result = handle.result;
    loop {
        tokio::select! {
            event = handle.events.recv() => {
                match event {
                    Some(event) => events.push(event),
                    None => break,
                }
            }
            messages = &mut result => {
                while let Some(event) = handle.events.recv().await {
                    events.push(event);
                }
                return (events, messages);
            }
        }
    }
    (events, result.await)
}

fn assistant_text(message: &AssistantMessage) -> String {
    message
        .content
        .iter()
        .filter_map(|part| match part {
            AssistantContent::Text(text) => Some(text.text.clone()),
            AssistantContent::ToolCall(_) => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

fn format_tool_args(tool_name: &str, args: &Value) -> String {
    let primary = match tool_name {
        "bash" => args.get("command").and_then(Value::as_str),
        "read" | "write" => args.get("path").and_then(Value::as_str),
        _ => None,
    };
    primary
        .map(single_line)
        .unwrap_or_else(|| serde_json::to_string(args).unwrap_or_else(|_| "{}".into()))
}

fn single_line(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

fn extract_first_bold(text: &str) -> Option<String> {
    let start = text.find("**")?.saturating_add(2);
    let rest = text.get(start..)?;
    let end = rest.find("**")?;
    let header = rest.get(..end)?.trim();
    (!header.is_empty()).then(|| header.to_string())
}

pub fn install_default_stream_fn() {
    use crate::backend::opencode::{OpenCodeStreamConfig, opencode_stream_fn};
    let client = reqwest::Client::new();
    let config = OpenCodeStreamConfig {
        client,
        output_limit: 8192,
    };
    set_default_stream_fn(Some(opencode_stream_fn(config)));
}

#[cfg(test)]
pub fn test_stream_fn() -> StreamFn {
    use crate::backend::agent::{
        AssistantMessageEvent, StopReason, TextContent, assistant_message, assistant_message_stream,
    };
    Arc::new(|_model, _context, _options, _cancel| {
        Box::pin(async move {
            let stream = assistant_message_stream();
            let message = assistant_message(
                vec![AssistantContent::Text(TextContent::new("mock reply"))],
                StopReason::Stop,
            );
            stream.push(AssistantMessageEvent::Done {
                reason: StopReason::Stop,
                message: message.clone(),
            });
            stream.end(Some(message));
            stream
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{
        AgentSession, build_system_prompt, extract_first_bold, format_tool_args,
        lowest_title_reasoning_variant, parse_generated_title, test_stream_fn,
        text_generation_model,
    };
    use crate::backend::agent::{
        AgentMessage, AssistantContent, AssistantMessageEvent, StopReason, StreamFn, TextContent,
        ToolCall, assistant_message, assistant_message_stream,
    };
    use crate::backend::opencode::types::{
        ProviderOptions, ReasoningVariant, ReasoningVariantKind,
    };
    use crate::config::RaidSettings;
    use crate::frontend::chat::{Role, ViewportState};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn system_prompt_identifies_the_current_workspace() {
        let prompt = build_system_prompt(std::path::Path::new("/tmp/raid-workspace"));

        assert!(prompt.ends_with("Current working directory: /tmp/raid-workspace"));
    }

    #[test]
    fn reasoning_status_uses_the_first_complete_bold_heading() {
        assert_eq!(extract_first_bold("still thinking"), None);
        assert_eq!(extract_first_bold("**Inspecting files"), None);
        assert_eq!(
            extract_first_bold("**Inspecting files**\nMore reasoning **later**").as_deref(),
            Some("Inspecting files")
        );
    }

    #[test]
    fn streamed_text_keeps_the_activity_indicator_visible() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut session = AgentSession::new(rt.handle().clone());
        session.activity_visible = true;
        let partial = assistant_message(
            vec![AssistantContent::Text(TextContent::new("hello"))],
            StopReason::Stop,
        );

        session.on_assistant_stream_event(&AssistantMessageEvent::TextDelta {
            content_index: 0,
            delta: "hello".into(),
            partial,
        });

        assert!(session.activity_visible);
    }

    #[test]
    fn interrupt_cancels_the_active_run() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut session = AgentSession::new(rt.handle().clone()).with_stream_fn(test_stream_fn());
        session.submit("hello".into());

        assert!(session.interrupt());
        assert!(session.cancel.is_cancelled());
    }

    #[test]
    fn tool_only_assistant_turn_does_not_create_an_empty_chat_row() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut session = AgentSession::new(rt.handle().clone());
        let mut chat = ViewportState::default();
        let message = assistant_message(
            vec![AssistantContent::ToolCall(ToolCall::new(
                "call-1",
                "bash",
                serde_json::json!({ "command": "ls -la" }),
            ))],
            StopReason::ToolUse,
        );

        session.on_message_start(&mut chat, AgentMessage::Assistant(message.clone()));
        session.on_message_end(&mut chat, AgentMessage::Assistant(message));

        assert!(chat.is_empty());
    }

    #[test]
    fn tool_headers_show_primary_arguments_instead_of_json() {
        assert_eq!(
            format_tool_args(
                "bash",
                &serde_json::json!({ "command": "find src\n-type f", "timeout": 30 }),
            ),
            "find src -type f"
        );
        assert_eq!(
            format_tool_args(
                "write",
                &serde_json::json!({ "path": "src/main.rs", "content": "secret body" }),
            ),
            "src/main.rs"
        );
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let id = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "raid-agent-session-test-{}-{id}",
                std::process::id()
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

    #[test]
    fn session_collects_assistant_reply() {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut chat = ViewportState::default();
        let mut session = AgentSession::new(rt.handle().clone()).with_stream_fn(test_stream_fn());
        session.submit("hello".into());
        session.drive_to_completion(&mut chat);
        assert_eq!(chat.last_role(), Some(Role::Assistant));
    }

    #[test]
    fn session_poll_receives_reply_without_awaiting_result() {
        use std::time::{Duration, Instant};

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut chat = ViewportState::default();
        let mut session = AgentSession::new(rt.handle().clone()).with_stream_fn(test_stream_fn());
        session.submit("hello".into());

        let deadline = Instant::now() + Duration::from_secs(1);
        while session.is_running() && Instant::now() < deadline {
            session.poll(&mut chat);
            std::thread::sleep(Duration::from_millis(1));
        }

        assert!(!session.is_running());
        assert_eq!(chat.last_role(), Some(Role::Assistant));
    }

    #[test]
    fn session_persists_user_and_assistant_messages() {
        let root = TestDir::new();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut chat = ViewportState::default();
        let mut session = AgentSession::new(rt.handle().clone())
            .with_stream_fn(test_stream_fn())
            .with_session_root(root.0.clone());

        session.submit("hello from storage".into());
        session.drive_to_completion(&mut chat);

        let path = session.session_path().expect("session path");
        let connection = rusqlite::Connection::open(path).expect("open session");
        let roles = connection
            .prepare("SELECT role FROM entries WHERE kind = 'message' ORDER BY sequence")
            .expect("prepare roles")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query roles")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect roles");
        assert_eq!(roles, ["user", "assistant"]);
    }

    #[test]
    fn generated_title_json_is_sanitized() {
        assert_eq!(
            parse_generated_title("```json\n{\"title\":\"Fix Session Resume.\"}\n```"),
            Some("Fix Session Resume".into())
        );
    }

    #[test]
    fn generated_title_extracts_wrapped_json() {
        assert_eq!(
            parse_generated_title("Here is the title:\n{\"title\":\"Initial Greeting\"}\nDone."),
            Some("Initial Greeting".into())
        );
    }

    #[test]
    fn generated_title_rejects_conversational_replies() {
        assert_eq!(
            parse_generated_title("Hey! What's up? How can I help you?"),
            None
        );
        assert_eq!(
            parse_generated_title("{\"title\":\"Greeting\",\"extra\":true}"),
            None
        );
    }

    #[test]
    fn title_requests_prefer_disabled_then_low_reasoning() {
        let variant = |id: &str, label: &str| ReasoningVariant {
            id: id.into(),
            label: label.into(),
            kind: ReasoningVariantKind::Effort,
            provider_options: ProviderOptions::default(),
        };
        let variants = vec![
            variant("effort:high", "high"),
            variant("effort:low", "low"),
            variant("effort:none", "none"),
        ];
        assert_eq!(
            lowest_title_reasoning_variant(&variants).map(|variant| variant.id.as_str()),
            Some("effort:none")
        );

        let variants = vec![
            variant("effort:max", "max"),
            variant("effort:low", "low"),
            variant("effort:high", "high"),
        ];
        assert_eq!(
            lowest_title_reasoning_variant(&variants).map(|variant| variant.id.as_str()),
            Some("effort:low")
        );
    }

    #[test]
    fn titles_use_the_independent_text_generation_model() {
        let settings: RaidSettings = serde_json::from_str(
            r#"{
                "default_provider":"chat-provider",
                "default_model":"chat-model",
                "default_api":"responses",
                "text_generation_provider":"text-provider",
                "text_generation_model":"text-model",
                "text_generation_api":"openai-compatible"
            }"#,
        )
        .expect("settings");

        let model = text_generation_model(&settings);
        assert_eq!(model.provider, "text-provider");
        assert_eq!(model.id, "text-model");
        assert_eq!(model.api, "openai-compatible");
    }

    #[test]
    fn generated_title_renames_the_active_database() {
        let root = TestDir::new();
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let stream_fn: StreamFn = Arc::new(|_model, context, options, _cancel| {
            Box::pin(async move {
                assert_eq!(context.system_prompt, None);
                assert_eq!(context.messages.len(), 1);
                assert_eq!(options.max_output_tokens, None);
                let prompt = context.messages[0]
                    .content
                    .as_ref()
                    .and_then(|value| value.as_str())
                    .expect("title prompt");
                assert!(prompt.contains("Return one JSON object with exactly one key:"));
                assert!(prompt.ends_with(
                    "User message:\nplease repair all the broken session storage paths"
                ));
                let stream = assistant_message_stream();
                let message = assistant_message(
                    vec![AssistantContent::Text(TextContent::new(
                        "{\"title\":\"Repair Session Storage\"}",
                    ))],
                    StopReason::Stop,
                );
                stream.push(AssistantMessageEvent::Done {
                    reason: StopReason::Stop,
                    message: message.clone(),
                });
                stream.end(Some(message));
                stream
            })
        });
        let mut session = AgentSession::new(rt.handle().clone())
            .with_stream_fn(stream_fn)
            .with_session_root(root.0.clone());
        session.start_session("please repair all the broken session storage paths");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        while session
            .title_task
            .as_ref()
            .is_some_and(|task| !task.is_finished())
            && std::time::Instant::now() < deadline
        {
            std::thread::yield_now();
        }
        session.poll_title();
        let store = session.store.as_ref().expect("session store");
        assert_eq!(
            store.metadata().expect("metadata").title,
            "Repair Session Storage"
        );
        assert!(
            store
                .path()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("repair-session-storage--")
        );
    }

    #[test]
    fn opening_a_session_restores_messages_and_chat() {
        let root = TestDir::new();
        let project = std::env::current_dir().expect("project path");
        let path = {
            let mut store = crate::backend::session::SessionStore::create_in(
                &root.0,
                &project,
                "old prompt snapshot",
                "opencode-go",
                "gpt-4.1-mini",
                "openai-compatible",
            )
            .expect("create store");
            store
                .append_message(&crate::backend::agent::AgentMessage::User(
                    crate::backend::agent::UserMessage::new("restored user"),
                ))
                .expect("user message");
            let assistant = assistant_message(
                vec![AssistantContent::Text(TextContent::new("restored reply"))],
                StopReason::Stop,
            );
            store
                .append_message(&crate::backend::agent::AgentMessage::Assistant(assistant))
                .expect("assistant message");
            store.path().to_path_buf()
        };
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime");
        let mut session = AgentSession::new(rt.handle().clone())
            .with_stream_fn(test_stream_fn())
            .with_session_root(root.0.clone());
        let mut chat = ViewportState::default();
        session.open_session(path, &mut chat).expect("open session");
        assert_eq!(session.context.messages.len(), 2);
        assert_eq!(chat.last_role(), Some(Role::Assistant));
    }
}
