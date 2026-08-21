mod agent;
mod connect;
mod layout;

pub use agent::install_default_stream_fn;

use crate::frontend::chat::{MarkdownCache, Role, ViewportState};
use crate::frontend::composer::{ComposerAction, ComposerState, ComposerWidget};
use crate::frontend::connect::{ConnectPanelStep, ConnectPanelWidget};
use crate::frontend::tools::ToolStatus;
use agent::AgentSession;
use connect::{ConnectAction, ConnectFlow};
use crossterm::event::{KeyCode, KeyEvent};
use layout::shell_layout;
use ratatui::{Frame, layout::Rect};

pub struct App {
    composer: ComposerState,
    chat: ViewportState,
    cache: MarkdownCache,
    last_chat_width: usize,
    last_chat_height: usize,
    agent: AgentSession,
    connect: ConnectFlow,
}

impl App {
    pub fn new(runtime: tokio::runtime::Handle) -> Self {
        Self::with_agent(AgentSession::new(runtime))
    }

    fn with_agent(agent: AgentSession) -> Self {
        Self {
            composer: ComposerState::default(),
            chat: ViewportState::default(),
            cache: MarkdownCache::default(),
            last_chat_width: 0,
            last_chat_height: 0,
            agent,
            connect: ConnectFlow::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn connect_active(&self) -> bool {
        self.connect.active()
    }

    #[cfg(test)]
    pub fn with_stream_fn(runtime: tokio::runtime::Handle, stream_fn: crate::backend::agent::StreamFn) -> Self {
        Self::with_agent(AgentSession::new(runtime).with_stream_fn(stream_fn))
    }

    pub fn handle_key(&mut self, key: KeyEvent, content_width: usize) -> AppAction {
        if self.connect.active() {
            return self.handle_connect_key(key, content_width);
        }

        match key.code {
            KeyCode::PageUp => {
                let width = self.last_chat_width.max(content_width).max(1);
                let view = self.last_chat_height.max(1);
                let height = self.chat.content_height(&mut self.cache, width);
                self.chat
                    .scroll_up(view.saturating_sub(1).max(1), height, view);
                AppAction::None
            }
            KeyCode::PageDown => {
                let view = self.last_chat_height.max(1);
                self.chat.scroll_down(view.saturating_sub(1).max(1));
                AppAction::None
            }
            _ => match self.composer.handle_key_with_width(key, content_width) {
                ComposerAction::Quit => AppAction::Quit,
                ComposerAction::Submit(message) => {
                    self.chat.push(Role::User, message.clone());
                    self.agent.submit(message);
                    AppAction::None
                }
                ComposerAction::Command { name, args } => {
                    if name == "connect" {
                        self.connect.start();
                        self.composer = ComposerState::default();
                        return AppAction::None;
                    }
                    let detail = if args.is_empty() { String::new() } else { args };
                    let summary = if detail.is_empty() {
                        format!("Ran /{name}")
                    } else {
                        format!("Ran /{name} {detail}")
                    };
                    let index = self.chat.start_tool(name, detail);
                    self.chat
                        .finish_tool(index, ToolStatus::Success, summary);
                    AppAction::None
                }
                ComposerAction::None => AppAction::None,
            },
        }
    }

    fn handle_connect_key(&mut self, key: KeyEvent, content_width: usize) -> AppAction {
        match self.connect.handle_key(key, content_width) {
            ConnectAction::None => AppAction::None,
            ConnectAction::Cancelled => AppAction::None,
            ConnectAction::Finished { summary } => {
                self.agent.reload_credentials();
                self.chat.push(Role::Assistant, summary);
                AppAction::None
            }
        }
    }

    pub fn insert_paste(&mut self, pasted: &str) {
        if self.connect.active() {
            self.connect.insert_paste(pasted);
            return;
        }
        self.composer.insert_paste(pasted);
    }

    pub fn tick(&mut self) {
        self.agent.poll(&mut self.chat);
    }

    pub fn draw(&mut self, frame: &mut Frame) -> usize {
        let area = frame.area();
        if area.width < 5 || area.height < 4 {
            return 0;
        }
        let padded = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height.saturating_sub(1),
        };

        if self.connect.active() {
            return self.draw_connect(frame, padded);
        }

        let layout = shell_layout(padded, &self.composer, 0);

        if layout.chat.height > 0 {
            let width = layout.chat.width as usize;
            self.last_chat_width = width;
            self.last_chat_height = layout.chat.height as usize;
            frame.render_widget(self.chat.widget(&mut self.cache, width.max(1)), layout.chat);
        }
        frame.render_widget(ComposerWidget::new(&self.composer), layout.composer);
        if let (Some(area), Some(widget)) = (layout.palette, self.composer.palette_widget()) {
            frame.render_widget(widget, area);
        }
        layout.content_width
    }

    fn draw_connect(&mut self, frame: &mut Frame, area: Rect) -> usize {
        let panel_height = self
            .connect
            .panel_height(area.width, area.height)
            .min(area.height);
        let panel = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(panel_height),
            width: area.width,
            height: panel_height,
        };
        if panel.y > area.y {
            let chat = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: panel.y.saturating_sub(area.y),
            };
            let width = chat.width as usize;
            self.last_chat_width = width;
            self.last_chat_height = chat.height as usize;
            frame.render_widget(self.chat.widget(&mut self.cache, width.max(1)), chat);
        }

        let panel_layout = crate::frontend::composer::padded_input_layout(panel);
        let header = self.connect.header_text();
        let widget = match self.connect.panel_step() {
            ConnectPanelStep::Provider => {
                ConnectPanelWidget::provider(&header, self.connect.palette_selected())
            }
            ConnectPanelStep::ApiKey => ConnectPanelWidget::api_key(
                &header,
                self.connect.label_text(),
                self.connect.footer_text().unwrap_or(""),
                self.connect.input(),
                panel_layout.wrap_width.max(1),
                panel.height.saturating_sub(5).max(1),
            ),
        };
        frame.render_widget(widget, panel);

        panel_layout.wrap_width.max(1)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum AppAction {
    None,
    Quit,
}

#[cfg(test)]
impl App {
    fn run_agent_to_end(&mut self) {
        self.agent.drive_to_completion(&mut self.chat);
    }
}

#[cfg(test)]
mod tests {
    use super::{App, AppAction};
    use crate::app::agent::test_stream_fn;
    use crate::frontend::chat::Role;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("runtime")
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn submit_appends_a_user_message() {
        let rt = runtime();
        let mut app = App::with_stream_fn(rt.handle().clone(), test_stream_fn());
        for character in "**hi**".chars() {
            app.handle_key(key(KeyCode::Char(character)), 40);
        }
        assert_eq!(app.handle_key(key(KeyCode::Enter), 40), AppAction::None);
        assert!(!app.chat.is_empty());
        app.run_agent_to_end();
        assert_eq!(app.chat.last_role(), Some(Role::Assistant));
    }

    #[test]
    fn slash_command_shows_up_in_the_timeline() {
        let rt = runtime();
        let mut app = App::with_stream_fn(rt.handle().clone(), test_stream_fn());
        app.insert_paste("/status");
        assert_eq!(app.handle_key(key(KeyCode::Enter), 40), AppAction::None);
        assert!(app.chat.contains_tool("status"));
        assert_eq!(app.chat.last_role(), None);
    }

    #[test]
    fn connect_command_opens_provider_picker() {
        let rt = runtime();
        let mut app = App::with_stream_fn(rt.handle().clone(), test_stream_fn());
        app.insert_paste("/connect");
        assert_eq!(app.handle_key(key(KeyCode::Enter), 40), AppAction::None);
        assert!(app.connect_active());
    }

    #[test]
    fn draw_shows_markdown_chat() {
        let rt = runtime();
        let mut app = App::with_stream_fn(rt.handle().clone(), test_stream_fn());
        app.insert_paste("**hi**");
        app.handle_key(key(KeyCode::Enter), 40);
        app.run_agent_to_end();

        let mut terminal = Terminal::new(TestBackend::new(48, 32)).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut screen = String::new();
        for y in 0..32 {
            for x in 0..48 {
                screen.push_str(buffer.cell((x, y)).unwrap().symbol());
            }
            screen.push('\n');
        }
        assert!(screen.contains("mock reply") || screen.contains("hi"));
        assert!(screen.contains('>'));
    }
}
