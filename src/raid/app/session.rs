use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::task::JoinHandle;

use crate::backend::session::{delete_session, SessionSummary};
use crate::frontend::composer::{ComposerAction, ComposerState};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionAction {
    None,
    Cancelled,
    Selected(PathBuf),
    Error(String),
}

#[derive(Debug, Default)]
pub struct SessionFlow {
    active: bool,
    sessions: Vec<SessionSummary>,
    filtered: Vec<usize>,
    selected: usize,
    status: String,
    delete_armed: Option<PathBuf>,
    load: Option<JoinHandle<Result<Vec<SessionSummary>, String>>>,
    current: Option<PathBuf>,
}

impl SessionFlow {
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn sessions(&self) -> &[SessionSummary] {
        &self.sessions
    }

    pub fn filtered(&self) -> &[usize] {
        &self.filtered
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn current(&self) -> Option<&Path> {
        self.current.as_deref()
    }

    pub fn start(&mut self, sessions: Vec<SessionSummary>, current: Option<&Path>) {
        self.active = true;
        self.sessions = sessions;
        self.filtered = (0..self.sessions.len()).collect();
        self.selected = 0;
        self.delete_armed = None;
        self.current = current.map(Path::to_path_buf);
        self.status = if self.sessions.is_empty() {
            "No saved sessions for this project.".into()
        } else {
            String::new()
        };
    }

    pub fn start_loading(
        &mut self,
        load: Option<JoinHandle<Result<Vec<SessionSummary>, String>>>,
        current: Option<&Path>,
    ) {
        *self = Self::default();
        self.active = true;
        self.load = load;
        self.current = current.map(Path::to_path_buf);
        self.status = if self.load.is_some() {
            "Loading saved sessions...".into()
        } else {
            "Session persistence is disabled.".into()
        };
    }

    pub fn poll(&mut self, runtime: &tokio::runtime::Handle) {
        let Some(load) = self.load.as_ref() else {
            return;
        };
        if !load.is_finished() {
            return;
        }
        let load = self.load.take().expect("checked session scan");
        match runtime.block_on(load) {
            Ok(Ok(sessions)) => {
                let current = self.current.take();
                self.start(sessions, current.as_deref());
            }
            Ok(Err(error)) => self.status = error,
            Err(error) => self.status = format!("Session scan failed: {error}"),
        }
    }

    pub fn apply_filter(&mut self, query: &str) {
        let query = query.trim().to_lowercase();
        self.filtered = self
            .sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| {
                query.is_empty()
                    || session.title.to_lowercase().contains(&query)
                    || session.id.to_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
        self.selected = 0;
        self.delete_armed = None;
        self.status.clear();
    }

    pub fn handle_key(
        &mut self,
        composer: &mut ComposerState,
        key: KeyEvent,
        content_width: usize,
    ) -> SessionAction {
        if !self.active || key.kind != KeyEventKind::Press {
            return SessionAction::None;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return self.cancel();
        }
        if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return self.delete_selected();
        }
        match key.code {
            KeyCode::Esc => self.cancel(),
            KeyCode::Up => {
                self.move_selected(-1);
                SessionAction::None
            }
            KeyCode::Down => {
                self.move_selected(1);
                SessionAction::None
            }
            KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => self.select_current(),
            _ => {
                let before = composer.text().to_string();
                match composer.handle_key_with_width(key, content_width) {
                    ComposerAction::Quit => self.cancel(),
                    ComposerAction::Submit(_) | ComposerAction::Command { .. } => SessionAction::None,
                    ComposerAction::None => {
                        if composer.text() != before {
                            self.apply_filter(composer.text());
                        }
                        SessionAction::None
                    }
                }
            }
        }
    }

    pub fn insert_paste(&mut self, composer: &mut ComposerState, pasted: &str) {
        composer.insert_paste(pasted);
        self.apply_filter(composer.text());
    }

    fn select_current(&mut self) -> SessionAction {
        let Some(index) = self.filtered.get(self.selected).copied() else {
            return SessionAction::None;
        };
        let Some(session) = self.sessions.get(index) else {
            return SessionAction::None;
        };
        if self.current.as_deref() == Some(session.path.as_path()) {
            return self.cancel();
        }
        if session.locked {
            self.status = "That session is open in another Raid process.".into();
            return SessionAction::None;
        }
        self.active = false;
        SessionAction::Selected(session.path.clone())
    }

    fn delete_selected(&mut self) -> SessionAction {
        let Some(index) = self.filtered.get(self.selected).copied() else {
            return SessionAction::None;
        };
        let Some(session) = self.sessions.get(index) else {
            return SessionAction::None;
        };
        if self.current.as_deref() == Some(session.path.as_path()) {
            self.status = "Start a new session before deleting the current one.".into();
            return SessionAction::None;
        }
        if session.locked {
            self.status = "Close the other Raid process before deleting this session.".into();
            return SessionAction::None;
        }
        if self.delete_armed.as_ref() != Some(&session.path) {
            self.delete_armed = Some(session.path.clone());
            self.status = "Press Ctrl+D again to move this session to trash.".into();
            return SessionAction::None;
        }
        let path = session.path.clone();
        match delete_session(&path) {
            Ok(()) => {
                self.sessions.remove(index);
                self.filtered = (0..self.sessions.len()).collect();
                self.selected = self.selected.min(self.filtered.len().saturating_sub(1));
                self.delete_armed = None;
                self.status = "Session moved to trash.".into();
                SessionAction::None
            }
            Err(error) => SessionAction::Error(error.to_string()),
        }
    }

    fn move_selected(&mut self, delta: isize) {
        if self.filtered.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(delta)
            .min(self.filtered.len() - 1);
        self.delete_armed = None;
        self.status.clear();
    }

    fn cancel(&mut self) -> SessionAction {
        self.active = false;
        self.delete_armed = None;
        SessionAction::Cancelled
    }
}

#[cfg(test)]
mod tests {
    use super::{SessionAction, SessionFlow};
    use crate::backend::session::SessionSummary;
    use crate::frontend::composer::ComposerState;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    fn summary(title: &str, model: &str, locked: bool) -> SessionSummary {
        SessionSummary {
            path: PathBuf::from(format!("/{title}.db")),
            id: format!("id-{title}"),
            title: title.into(),
            updated_at: 1,
            message_count: 2,
            current_provider: "provider".into(),
            current_model: model.into(),
            locked,
        }
    }

    #[test]
    fn filters_by_title_and_id() {
        let mut flow = SessionFlow::default();
        flow.start(
            vec![
                summary("Repair storage", "alpha", false),
                summary("Polish picker", "beta", false),
            ],
            None,
        );
        flow.apply_filter("repair");
        assert_eq!(flow.filtered(), &[0]);
        flow.apply_filter("id-polish");
        assert_eq!(flow.filtered(), &[1]);
    }

    #[test]
    fn enter_resumes_the_selected_unlocked_session() {
        let mut flow = SessionFlow::default();
        flow.start(vec![summary("Repair", "alpha", false)], None);
        let action = flow.handle_key(
            &mut ComposerState::default(),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            40,
        );
        assert_eq!(action, SessionAction::Selected(PathBuf::from("/Repair.db")));
    }

    #[test]
    fn current_session_stays_visible_and_enter_closes_the_picker() {
        let mut flow = SessionFlow::default();
        let current = PathBuf::from("/Current.db");
        flow.start(vec![summary("Current", "alpha", true)], Some(&current));

        assert_eq!(flow.sessions().len(), 1);
        assert_eq!(flow.current(), Some(current.as_path()));
        let action = flow.handle_key(
            &mut ComposerState::default(),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            40,
        );

        assert_eq!(action, SessionAction::Cancelled);
        assert!(!flow.active());
    }

    #[test]
    fn current_session_cannot_be_deleted() {
        let mut flow = SessionFlow::default();
        let current = PathBuf::from("/Current.db");
        flow.start(vec![summary("Current", "alpha", true)], Some(&current));

        let action = flow.handle_key(
            &mut ComposerState::default(),
            KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            40,
        );

        assert_eq!(action, SessionAction::None);
        assert!(flow.status().contains("current one"));
    }

    #[test]
    fn locked_sessions_cannot_be_resumed() {
        let mut flow = SessionFlow::default();
        flow.start(vec![summary("Busy", "alpha", true)], None);
        let action = flow.handle_key(
            &mut ComposerState::default(),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            40,
        );
        assert_eq!(action, SessionAction::None);
        assert!(flow.status().contains("another Raid process"));
    }
}
