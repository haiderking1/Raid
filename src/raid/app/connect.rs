use crate::config::{save_connection, PROVIDERS};
use crate::frontend::composer::{ComposerAction, ComposerState};
use crate::frontend::connect::panel_height;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Step {
    #[default]
    Provider,
    ApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectAction {
    None,
    Finished { summary: String },
    Cancelled,
}


#[derive(Debug, Default)]
pub struct ConnectFlow {
    active: bool,
    step: Step,
    provider_selected: usize,
    provider: Option<&'static crate::config::ConnectProvider>,
    input: ComposerState,
}

impl ConnectFlow {
    pub fn start(&mut self) {
        *self = Self {
            active: true,
            step: Step::Provider,
            provider_selected: 0,
            provider: None,
            input: ComposerState::default(),
        };
    }

    pub fn active(&self) -> bool {
        self.active
    }

    pub fn panel_step(&self) -> crate::frontend::connect::ConnectPanelStep {
        match self.step {
            Step::Provider => crate::frontend::connect::ConnectPanelStep::Provider,
            Step::ApiKey => crate::frontend::connect::ConnectPanelStep::ApiKey,
        }
    }

    pub fn panel_height(&self, area_width: u16, max_height: u16) -> u16 {
        panel_height(
            self.panel_step(),
            area_width,
            max_height,
            &self.input,
        )
    }

    pub fn footer_text(&self) -> Option<&'static str> {
        match self.step {
            Step::Provider => None,
            Step::ApiKey => Some("(shift+enter newline, enter submit, esc cancel)"),
        }
    }

    pub fn header_text(&self) -> String {
        match self.step {
            Step::Provider => "Select provider to configure:".into(),
            Step::ApiKey => self
                .provider
                .map(|provider| format!("Connect to {}", provider.label))
                .unwrap_or_else(|| "Connect".into()),
        }
    }

    pub fn label_text(&self) -> &'static str {
        "Enter OpenCode API key"
    }

    pub fn palette_selected(&self) -> usize {
        self.provider_selected
    }

    pub fn input(&self) -> &ComposerState {
        &self.input
    }

    pub fn handle_key(&mut self, key: KeyEvent, content_width: usize) -> ConnectAction {
        if !self.active || key.kind != KeyEventKind::Press {
            return ConnectAction::None;
        }
        match self.step {
            Step::Provider => self.handle_provider_key(key),
            Step::ApiKey => self.handle_api_key_key(key, content_width),
        }
    }

    pub fn insert_paste(&mut self, pasted: &str) {
        if !self.active || self.step != Step::ApiKey {
            return;
        }
        self.input.insert_paste(pasted);
    }

    fn handle_provider_key(&mut self, key: KeyEvent) -> ConnectAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return self.cancel();
        }
        match key.code {
            KeyCode::Esc => self.cancel(),
            KeyCode::Up => {
                if !PROVIDERS.is_empty() {
                    let next = self.provider_selected as isize - 1;
                    self.provider_selected = next.rem_euclid(PROVIDERS.len() as isize) as usize;
                }
                ConnectAction::None
            }
            KeyCode::Down => {
                if !PROVIDERS.is_empty() {
                    let next = self.provider_selected as isize + 1;
                    self.provider_selected = next.rem_euclid(PROVIDERS.len() as isize) as usize;
                }
                ConnectAction::None
            }
            KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                let provider = &PROVIDERS[self.provider_selected];
                self.provider = Some(provider);
                self.step = Step::ApiKey;
                self.input = ComposerState::default();
                ConnectAction::None
            }
            _ => ConnectAction::None,
        }
    }

    fn handle_api_key_key(&mut self, key: KeyEvent, content_width: usize) -> ConnectAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return self.cancel();
        }
        if key.code == KeyCode::Esc {
            self.step = Step::Provider;
            self.provider = None;
            self.input = ComposerState::default();
            return ConnectAction::None;
        }
        if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
            return self.submit_api_key();
        }

        match self.input.handle_key_with_width(key, content_width) {
            ComposerAction::Quit => self.cancel(),
            ComposerAction::Submit(_) => ConnectAction::None,
            _ => ConnectAction::None,
        }
    }

    fn submit_api_key(&mut self) -> ConnectAction {
        let Some(provider) = self.provider else {
            return self.cancel();
        };
        let key = self.input.text().trim();
        if key.is_empty() {
            return ConnectAction::None;
        }
        match save_connection(provider.id, key) {
            Ok(()) => {
                let summary = format!(
                    "Connected to {}. Credentials saved to {}",
                    provider.label,
                    crate::config::auth_path().display()
                );
                self.active = false;
                ConnectAction::Finished { summary }
            }
            Err(error) => {
                self.active = false;
                ConnectAction::Finished {
                    summary: format!("Failed to save credentials: {error}"),
                }
            }
        }
    }

    fn cancel(&mut self) -> ConnectAction {
        self.active = false;
        ConnectAction::Cancelled
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn shift_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    #[test]
    fn provider_step_advances_to_api_key_entry() {
        let mut flow = ConnectFlow::default();
        flow.start();
        assert_eq!(flow.handle_key(key(KeyCode::Enter), 80), ConnectAction::None);
        assert_eq!(flow.step, Step::ApiKey);
        assert!(flow.provider.is_some());
    }

    #[test]
    fn shift_enter_inserts_newline_in_api_key_input() {
        let mut flow = ConnectFlow::default();
        flow.start();
        assert_eq!(flow.handle_key(key(KeyCode::Enter), 80), ConnectAction::None);
        flow.handle_key(key(KeyCode::Char('a')), 80);
        flow.handle_key(shift_key(KeyCode::Enter), 80);
        flow.handle_key(key(KeyCode::Char('b')), 80);
        assert_eq!(flow.input.text(), "a\nb");
    }

    #[test]
    fn api_key_panel_grows_with_wrapped_input_lines() {
        let mut flow = ConnectFlow::default();
        flow.start();
        flow.handle_key(key(KeyCode::Enter), 80);
        flow.input.insert_paste("one\ntwo");
        assert!(flow.panel_height(80, 24) > 6);
    }
}
