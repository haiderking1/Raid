mod layout;

use crate::frontend::chat::{MarkdownCache, Role, ViewportState};
use crate::frontend::composer::{ComposerAction, ComposerState, ComposerWidget};
use crate::frontend::tools::{ToolLog, ToolStatus};
use crossterm::event::{KeyCode, KeyEvent};
use layout::{THINKING_RESERVE, shell_layout};
use ratatui::{Frame, layout::Rect};

#[derive(Default)]
pub struct App {
    composer: ComposerState,
    chat: ViewportState,
    tools: ToolLog,
    cache: MarkdownCache,
    last_chat_width: usize,
    last_chat_height: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub enum AppAction {
    None,
    Quit,
}

impl App {
    pub fn handle_key(&mut self, key: KeyEvent, content_width: usize) -> AppAction {
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
                    self.chat.push(Role::User, message);
                    AppAction::None
                }
                ComposerAction::Command { name, args } => {
                    let detail = if args.is_empty() { String::new() } else { args };
                    let index = self.tools.start(name, detail);
                    self.tools.finish(index, ToolStatus::Success);
                    AppAction::None
                }
                ComposerAction::None => AppAction::None,
            },
        }
    }

    pub fn insert_paste(&mut self, pasted: &str) {
        self.composer.insert_paste(pasted);
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

        let tools_height = self
            .tools
            .desired_height(padded.height.saturating_sub(3 + THINKING_RESERVE));
        let layout = shell_layout(padded, &self.composer, tools_height);

        if layout.chat.height > 0 {
            let width = layout.chat.width as usize;
            self.last_chat_width = width;
            self.last_chat_height = layout.chat.height as usize;
            frame.render_widget(self.chat.widget(&mut self.cache, width.max(1)), layout.chat);
        }
        if let Some(area) = layout.tools {
            frame.render_widget(self.tools.widget(), area);
        }
        frame.render_widget(ComposerWidget::new(&self.composer), layout.composer);
        if let (Some(area), Some(widget)) = (layout.palette, self.composer.palette_widget()) {
            frame.render_widget(widget, area);
        }
        if let Some(cursor) = ComposerWidget::cursor_position(layout.composer, &self.composer) {
            frame.set_cursor_position(cursor);
        }
        layout.content_width
    }
}

#[cfg(test)]
mod tests {
    use super::{App, AppAction};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn submit_appends_a_user_message() {
        let mut app = App::default();
        for character in "**hi**".chars() {
            app.handle_key(key(KeyCode::Char(character)), 40);
        }
        assert_eq!(app.handle_key(key(KeyCode::Enter), 40), AppAction::None);
        assert!(!app.chat.is_empty());
    }

    #[test]
    fn slash_command_shows_up_in_the_tools_pane() {
        let mut app = App::default();
        app.insert_paste("/status");
        assert_eq!(app.handle_key(key(KeyCode::Enter), 40), AppAction::None);
        assert!(!app.tools.is_empty());
        assert!(app.chat.is_empty());
    }

    #[test]
    fn draw_shows_markdown_chat_and_the_tools_pane() {
        let mut app = App::default();
        app.insert_paste("**hi**");
        app.handle_key(key(KeyCode::Enter), 40);
        app.insert_paste("/status");
        app.handle_key(key(KeyCode::Enter), 40);

        let mut terminal = Terminal::new(TestBackend::new(48, 16)).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let mut screen = String::new();
        for y in 0..16 {
            for x in 0..48 {
                screen.push_str(buffer.cell((x, y)).unwrap().symbol());
            }
            screen.push('\n');
        }
        assert!(screen.contains("hi"));
        assert!(!screen.contains("**"));
        assert!(!screen.contains("you"));
        assert!(screen.contains("status"));
        assert!(screen.contains("done"));
        assert!(screen.contains('>'));

        let mut first_row = String::new();
        for x in 0..48 {
            first_row.push_str(buffer.cell((x, 0)).unwrap().symbol());
        }
        assert!(first_row.contains('>'));
        assert!(first_row.contains("hi"));
    }
}
