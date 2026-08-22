mod agent;
mod connect;
mod layout;
mod model;
mod session;

pub use agent::install_default_stream_fn;

use crate::frontend::activity::ActivityIndicator;
use crate::frontend::chat::{MarkdownCache, Role, ViewportState};
use crate::frontend::composer::{ComposerAction, ComposerState, ComposerWidget};
use crate::frontend::connect::{ConnectPanelStep, ConnectPanelWidget};
use crate::frontend::model::{ModelPaletteWidget, model_input_wrap_width, model_palette_height};
use crate::frontend::session::{
    SessionPaletteWidget, session_input_wrap_width, session_palette_height,
};
use crate::frontend::status_line::StatusLineWidget;
use agent::AgentSession;
use connect::{ConnectAction, ConnectFlow};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use layout::shell_layout;
use model::{ModelAction, ModelFlow, ModelTarget};
use ratatui::{Frame, layout::Rect};
use session::{SessionAction, SessionFlow};

pub struct App {
    composer: ComposerState,
    chat: ViewportState,
    cache: MarkdownCache,
    activity: ActivityIndicator,
    last_chat_width: usize,
    last_chat_height: usize,
    agent: AgentSession,
    connect: ConnectFlow,
    model: ModelFlow,
    session: SessionFlow,
    runtime: tokio::runtime::Handle,
}

const MOUSE_SCROLL_LINES: usize = 3;

#[derive(Debug, Default)]
pub struct LaunchOptions {
    pub continue_session: bool,
    pub resume: bool,
    pub no_session: bool,
    pub session: Option<std::path::PathBuf>,
}

impl App {
    pub fn new(runtime: tokio::runtime::Handle) -> Self {
        Self::with_agent(AgentSession::new(runtime.clone()), runtime)
    }

    pub fn new_with_launch(runtime: tokio::runtime::Handle, launch: LaunchOptions) -> Self {
        let mut app = Self::new(runtime);
        if launch.no_session {
            app.agent.disable_persistence();
        }
        if let Some(path) = launch.session {
            if let Err(error) = app.agent.open_session(path, &mut app.chat) {
                app.chat.push(Role::Assistant, error);
            }
        } else if launch.continue_session {
            match app.agent.open_most_recent(&mut app.chat) {
                Ok(true) | Ok(false) => {}
                Err(error) => app.chat.push(Role::Assistant, error),
            }
        }
        if launch.resume {
            app.start_session_picker();
        }
        app
    }

    fn with_agent(agent: AgentSession, runtime: tokio::runtime::Handle) -> Self {
        Self {
            composer: ComposerState::default(),
            chat: ViewportState::default(),
            cache: MarkdownCache::default(),
            activity: ActivityIndicator::default(),
            last_chat_width: 0,
            last_chat_height: 0,
            agent,
            connect: ConnectFlow::default(),
            model: ModelFlow::default(),
            session: SessionFlow::default(),
            runtime,
        }
    }

    #[cfg(test)]
    pub(crate) fn connect_active(&self) -> bool {
        self.connect.active()
    }

    #[cfg(test)]
    pub(crate) fn model_active(&self) -> bool {
        self.model.active()
    }

    #[cfg(test)]
    pub(crate) fn session_active(&self) -> bool {
        self.session.active()
    }

    #[cfg(test)]
    pub fn with_stream_fn(
        runtime: tokio::runtime::Handle,
        stream_fn: crate::backend::agent::StreamFn,
    ) -> Self {
        Self::with_agent(
            AgentSession::new(runtime.clone()).with_stream_fn(stream_fn),
            runtime,
        )
    }

    pub fn handle_key(&mut self, key: KeyEvent, content_width: usize) -> AppAction {
        if self.connect.active() {
            return self.handle_connect_key(key, content_width);
        }
        if self.model.active() {
            return self.handle_model_key(key, content_width);
        }
        if self.session.active() {
            return self.handle_session_key(key, content_width);
        }

        if key.code == KeyCode::Esc && self.agent.interrupt() {
            return AppAction::None;
        }
        if key.code == KeyCode::Char('r') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.start_session_picker();
            return AppAction::None;
        }
        if key.code == KeyCode::Char('o') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.chat.toggle_tool_details();
            return AppAction::None;
        }
        if key.code == KeyCode::Char('n') && key.modifiers.contains(KeyModifiers::CONTROL) {
            if let Err(error) = self.agent.new_session(&mut self.chat) {
                self.chat.push(Role::Assistant, error);
            } else {
                self.composer = ComposerState::default();
                self.cache = MarkdownCache::default();
            }
            return AppAction::None;
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
                    match name.as_str() {
                        "connect" => {
                            self.connect.start();
                            self.composer = ComposerState::default();
                        }
                        "model" => {
                            self.model.start(&self.runtime, ModelTarget::Chat);
                            self.composer = ComposerState::default();
                            self.model.apply_filter(self.composer.text());
                        }
                        "text-model" => {
                            self.model.start(&self.runtime, ModelTarget::TextGeneration);
                            self.composer = ComposerState::default();
                            self.model.apply_filter(self.composer.text());
                        }
                        "compact" => {
                            self.composer = ComposerState::default();
                            if let Err(error) = self.agent.compact(args) {
                                self.chat.push(Role::Assistant, error);
                            }
                        }
                        "new" => {
                            if let Err(error) = self.agent.new_session(&mut self.chat) {
                                self.chat.push(Role::Assistant, error);
                            } else {
                                self.composer = ComposerState::default();
                                self.cache = MarkdownCache::default();
                            }
                        }
                        "resume" => self.start_session_picker(),
                        _ => {}
                    }
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
                if !summary.is_empty() {
                    self.chat.push(Role::Assistant, summary);
                }
                AppAction::None
            }
        }
    }

    fn handle_model_key(&mut self, key: KeyEvent, content_width: usize) -> AppAction {
        let width = model_input_wrap_width(self.last_chat_width.max(content_width) as u16 + 3)
            .max(content_width);
        match self.model.handle_key(&mut self.composer, key, width) {
            ModelAction::None => AppAction::None,
            ModelAction::Cancelled => {
                self.composer = ComposerState::default();
                AppAction::None
            }
            ModelAction::Selected { target } => {
                if target == ModelTarget::Chat {
                    self.agent.reload_credentials();
                } else {
                    self.agent.retry_session_title();
                }
                self.composer = ComposerState::default();
                AppAction::None
            }
            ModelAction::Error { message } => {
                self.composer = ComposerState::default();
                self.chat.push(Role::Assistant, message);
                AppAction::None
            }
        }
    }

    fn handle_session_key(&mut self, key: KeyEvent, content_width: usize) -> AppAction {
        let width = session_input_wrap_width(self.last_chat_width.max(content_width) as u16 + 3)
            .max(content_width);
        match self.session.handle_key(&mut self.composer, key, width) {
            SessionAction::None => AppAction::None,
            SessionAction::Cancelled => {
                self.composer = ComposerState::default();
                AppAction::None
            }
            SessionAction::Selected(path) => {
                self.composer = ComposerState::default();
                match self.agent.open_session(path, &mut self.chat) {
                    Ok(()) => self.cache = MarkdownCache::default(),
                    Err(error) => self.chat.push(Role::Assistant, error),
                }
                AppAction::None
            }
            SessionAction::Error(error) => {
                self.chat.push(Role::Assistant, error);
                AppAction::None
            }
        }
    }

    fn start_session_picker(&mut self) {
        if self.agent.is_running() {
            self.chat.push(
                Role::Assistant,
                "Wait for the current response before opening sessions.".into(),
            );
            return;
        }
        self.session.start_loading(
            self.agent.scan_sessions(),
            self.agent.current_session_path(),
        );
        self.composer = ComposerState::default();
    }

    pub fn insert_paste(&mut self, pasted: &str) {
        if self.connect.active() {
            self.connect.insert_paste(pasted);
            return;
        }
        if self.model.active() {
            self.model.insert_paste(&mut self.composer, pasted);
            return;
        }
        if self.session.active() {
            self.session.insert_paste(&mut self.composer, pasted);
            return;
        }
        self.composer.insert_paste(pasted);
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        if self.connect.active()
            || self.model.active()
            || self.session.active()
            || mouse.column as usize >= self.last_chat_width
            || mouse.row as usize >= self.last_chat_height
        {
            return;
        }

        match mouse.kind {
            MouseEventKind::ScrollUp => {
                let width = self.last_chat_width.max(1);
                let view = self.last_chat_height.max(1);
                let height = self.chat.content_height(&mut self.cache, width);
                self.chat.scroll_up(MOUSE_SCROLL_LINES, height, view);
            }
            MouseEventKind::ScrollDown => self.chat.scroll_down(MOUSE_SCROLL_LINES),
            _ => {}
        }
    }

    pub fn tick(&mut self) {
        self.model.poll(&self.runtime);
        self.agent.poll(&mut self.chat);
        if self.session.active() {
            let current = self
                .agent
                .current_session_path()
                .map(|path| path.to_path_buf());
            if self.session.current() != current.as_deref() {
                self.session
                    .start_loading(self.agent.scan_sessions(), current.as_deref());
            }
        }
        self.session.poll(&self.runtime);
    }

    pub fn draw(&mut self, frame: &mut Frame) -> usize {
        self.activity.sync(self.agent.activity_header());
        let area = frame.area();
        if area.width < 5 || area.height < 4 {
            return 0;
        }
        let body = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height.saturating_sub(1),
        };
        let status = Rect {
            x: area.x,
            y: area.bottom().saturating_sub(1),
            width: area.width,
            height: 1,
        };

        let content_width = if self.connect.active() {
            self.draw_connect(frame, body)
        } else if self.model.active() {
            self.draw_model(frame, body)
        } else if self.session.active() {
            self.draw_session(frame, body)
        } else {
            let layout = shell_layout(
                body,
                &self.composer,
                0,
                self.activity.widget().is_some(),
            );

            if layout.chat.height > 0 {
                let width = layout.chat.width as usize;
                self.last_chat_width = width;
                self.last_chat_height = layout.chat.height as usize;
                frame.render_widget(self.chat.widget(&mut self.cache, width.max(1)), layout.chat);
            }
            frame.render_widget(ComposerWidget::new(&self.composer), layout.composer);
            if let (Some(area), Some(widget)) = (layout.thinking, self.activity.widget()) {
                frame.render_widget(widget, area);
            }
            if let (Some(area), Some(widget)) = (layout.palette, self.composer.palette_widget()) {
                frame.render_widget(widget, area);
            }
            layout.content_width
        };

        let (context_tokens, context_limit) = self.agent.context_usage();
        frame.render_widget(
            StatusLineWidget::new(
                self.agent.model_id(),
                context_tokens,
                context_limit,
                self.agent.thinking_level(),
                self.agent.project_path(),
            ),
            status,
        );
        content_width
    }

    fn draw_model(&mut self, frame: &mut Frame, area: Rect) -> usize {
        let palette_height =
            model_palette_height(self.model.filtered().len(), area.height.saturating_sub(2));
        let palette = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(palette_height),
            width: area.width,
            height: palette_height,
        };
        if palette.y > area.y {
            let chat = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: palette.y.saturating_sub(area.y),
            };
            let width = chat.width as usize;
            self.last_chat_width = width;
            self.last_chat_height = chat.height as usize;
            frame.render_widget(self.chat.widget(&mut self.cache, width.max(1)), chat);
        }

        frame.render_widget(
            ModelPaletteWidget::new(
                &self.composer,
                self.model.models(),
                self.model.filtered(),
                self.model.selected(),
                self.model.status(),
            ),
            palette,
        );
        model_input_wrap_width(palette.width)
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
                panel.height.saturating_sub(5).max(1),
            ),
        };
        frame.render_widget(widget, panel);

        crate::frontend::connect::connect_input_wrap_width(panel.width)
    }

    fn draw_session(&mut self, frame: &mut Frame, area: Rect) -> usize {
        let palette_height =
            session_palette_height(self.session.filtered().len(), area.height.saturating_sub(2));
        let palette = Rect {
            x: area.x,
            y: area.y + area.height.saturating_sub(palette_height),
            width: area.width,
            height: palette_height,
        };
        if palette.y > area.y {
            let chat = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: palette.y.saturating_sub(area.y),
            };
            let width = chat.width as usize;
            self.last_chat_width = width;
            self.last_chat_height = chat.height as usize;
            frame.render_widget(self.chat.widget(&mut self.cache, width.max(1)), chat);
        }
        frame.render_widget(
            SessionPaletteWidget::new(
                &self.composer,
                self.session.sessions(),
                self.session.filtered(),
                self.session.selected(),
                self.session.status(),
                self.session.current(),
            ),
            palette,
        );
        session_input_wrap_width(palette.width)
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
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
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

    fn mouse(kind: MouseEventKind) -> MouseEvent {
        MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        }
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
    fn connect_command_opens_provider_picker() {
        let rt = runtime();
        let mut app = App::with_stream_fn(rt.handle().clone(), test_stream_fn());
        app.insert_paste("/connect");
        assert_eq!(app.handle_key(key(KeyCode::Enter), 40), AppAction::None);
        assert!(app.connect_active());
    }

    #[test]
    fn model_command_opens_interactive_picker() {
        let rt = runtime();
        let mut app = App::with_stream_fn(rt.handle().clone(), test_stream_fn());
        app.insert_paste("/model");
        assert_eq!(app.handle_key(key(KeyCode::Enter), 40), AppAction::None);
        assert!(app.model_active());
        assert_eq!(app.chat.last_role(), None);
    }

    #[test]
    fn text_model_command_opens_interactive_picker() {
        let rt = runtime();
        let mut app = App::with_stream_fn(rt.handle().clone(), test_stream_fn());
        app.insert_paste("/text-model");
        assert_eq!(app.handle_key(key(KeyCode::Enter), 40), AppAction::None);
        assert!(app.model_active());
        assert_eq!(app.chat.last_role(), None);
    }

    #[test]
    fn compact_command_reports_when_the_session_is_empty() {
        let rt = runtime();
        let mut app = App::with_stream_fn(rt.handle().clone(), test_stream_fn());

        app.insert_paste("/compact");
        assert_eq!(app.handle_key(key(KeyCode::Enter), 40), AppAction::None);

        assert_eq!(app.chat.last_role(), Some(Role::Assistant));
        assert!(!app.agent.is_running());
    }

    #[test]
    fn new_command_starts_with_an_empty_chat() {
        let rt = runtime();
        let mut app = App::with_stream_fn(rt.handle().clone(), test_stream_fn());
        app.chat.push(Role::User, "old session".into());

        app.insert_paste("/new");
        assert_eq!(app.handle_key(key(KeyCode::Enter), 40), AppAction::None);

        assert!(app.chat.is_empty());
        assert!(!app.session_active());
    }

    #[test]
    fn resume_command_opens_session_picker() {
        let rt = runtime();
        let mut app = App::with_stream_fn(rt.handle().clone(), test_stream_fn());

        app.insert_paste("/resume");
        assert_eq!(app.handle_key(key(KeyCode::Enter), 40), AppAction::None);

        assert!(app.session_active());
        assert_eq!(app.session.status(), "Session persistence is disabled.");
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

    #[test]
    fn mouse_wheel_moves_three_rows_per_event() {
        let rt = runtime();
        let mut app = App::with_stream_fn(rt.handle().clone(), test_stream_fn());
        app.last_chat_width = 30;
        app.last_chat_height = 4;
        for index in 0..8 {
            app.chat.push(Role::User, format!("message {index}"));
        }

        app.handle_mouse(mouse(MouseEventKind::ScrollUp));
        assert_eq!(app.chat.scroll_from_bottom(), 3);
        app.handle_mouse(mouse(MouseEventKind::ScrollDown));
        assert_eq!(app.chat.scroll_from_bottom(), 0);
    }

    #[test]
    fn ctrl_o_toggles_long_tool_results() {
        let rt = runtime();
        let mut app = App::with_stream_fn(rt.handle().clone(), test_stream_fn());
        let index = app.chat.start_tool("bash", "ls -la");
        app.chat
            .finish_tool(index, crate::frontend::tools::ToolStatus::Success, ".git\nsrc");

        app.handle_key(
            KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL),
            40,
        );

        assert!(app.chat.tool_details_expanded());
    }
}
