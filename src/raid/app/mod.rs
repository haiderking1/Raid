mod demo;
mod layout;

use crate::frontend::chat::{MarkdownCache, Role, ViewportState};
use crate::frontend::composer::{ComposerAction, ComposerState, ComposerWidget};
use crate::frontend::tools::ToolStatus;
use crossterm::event::{KeyCode, KeyEvent};
use demo::Demo;
use layout::shell_layout;
use ratatui::{Frame, layout::Rect};

#[derive(Default)]
pub struct App {
    composer: ComposerState,
    chat: ViewportState,
    cache: MarkdownCache,
    last_chat_width: usize,
    last_chat_height: usize,
    demo: Option<Demo>,
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
                    self.demo = Some(demo::Demo::new());
                    AppAction::None
                }
                ComposerAction::Command { name, args } => {
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
        if let Some(cursor) = ComposerWidget::cursor_position(layout.composer, &self.composer) {
            frame.set_cursor_position(cursor);
        }
        layout.content_width
    }
}

#[cfg(test)]
mod tests {
    use super::{App, AppAction};
    use crate::frontend::chat::Role;
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
        app.run_demo_to_end();
        assert_eq!(app.chat.last_role(), Some(Role::Assistant));
        assert!(app.chat.contains_tool("read"));
        assert!(app.chat.contains_tool("bash"));
    }

    #[test]
    fn slash_command_shows_up_in_the_timeline() {
        let mut app = App::default();
        app.insert_paste("/status");
        assert_eq!(app.handle_key(key(KeyCode::Enter), 40), AppAction::None);
        assert!(app.chat.contains_tool("status"));
        assert_eq!(app.chat.last_role(), None);
    }

    #[test]
    fn draw_shows_markdown_chat_and_inline_tools() {
        let mut app = App::default();
        app.insert_paste("**hi**");
        app.handle_key(key(KeyCode::Enter), 40);
        app.run_demo_to_end();

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
        assert!(screen.contains("DefaultTerminal"));
        assert!(screen.contains("Read("));
        assert!(screen.contains("Bash("));
        assert!(screen.contains("└"));
        assert!(screen.contains("ctrl+r"));
        assert!(screen.contains('>'));
        assert!(screen.contains("agent loop"));
        let inspect = screen.find("inspect").expect("inspect");
        let read = screen.find("Read(").expect("Read");
        let bash = screen.find("Bash(").expect("Bash");
        let wrap_up = screen.find("Done").expect("Done");
        assert!(inspect < read);
        assert!(read < bash);
        assert!(bash < wrap_up);
    }
}
