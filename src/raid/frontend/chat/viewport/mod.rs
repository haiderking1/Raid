use super::cache::MarkdownCache;
use crate::frontend::clip::render_clipped;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::Widget,
};

const USER_BAR: Color = Color::Rgb(48, 48, 50);
const USER_FG: Color = Color::Rgb(228, 228, 228);
const BULLET: Color = Color::Rgb(245, 245, 245);
const USER_PREFIX: &str = "> ";
const ASSISTANT_PREFIX: &str = "● ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    #[cfg_attr(not(test), expect(dead_code))]
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    pub role: Role,
    pub body: String,
}

#[derive(Debug, Default)]
pub struct ViewportState {
    messages: Vec<ChatMessage>,
    scroll_from_bottom: usize,
}

impl ViewportState {
    pub fn push(&mut self, role: Role, body: String) {
        self.messages.push(ChatMessage { role, body });
        self.scroll_from_bottom = 0;
    }

    #[cfg_attr(not(test), expect(dead_code))]
    pub fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }

    pub fn scroll_up(&mut self, page: usize, content_height: usize, view_height: usize) {
        let max_scroll = content_height.saturating_sub(view_height.max(1));
        self.scroll_from_bottom = (self.scroll_from_bottom + page.max(1)).min(max_scroll);
    }

    pub fn scroll_down(&mut self, page: usize) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(page.max(1));
    }

    pub fn content_height(&self, cache: &mut MarkdownCache, width: usize) -> usize {
        self.rendered_rows(cache, width).len()
    }

    pub fn widget(&self, cache: &mut MarkdownCache, width: usize) -> ViewportWidget {
        ViewportWidget {
            rows: self.rendered_rows(cache, width),
            scroll_from_bottom: self.scroll_from_bottom,
        }
    }

    fn rendered_rows(&self, cache: &mut MarkdownCache, width: usize) -> Vec<RenderedRow> {
        let mut rows = Vec::new();
        for message in &self.messages {
            let prefix_width = Line::from(prefix(message.role, true)).width();
            let body_width = width.saturating_sub(prefix_width).max(1);
            let mut body = cache.lines(&message.body, body_width).to_vec();
            if body.is_empty() {
                body.push(Line::default());
            }
            for (index, line) in body.into_iter().enumerate() {
                rows.push(RenderedRow::Line {
                    role: message.role,
                    first: index == 0,
                    line,
                });
            }
            rows.push(RenderedRow::Gap);
        }
        if matches!(rows.last(), Some(RenderedRow::Gap)) {
            rows.pop();
        }
        rows
    }
}

enum RenderedRow {
    Line {
        role: Role,
        first: bool,
        line: Line<'static>,
    },
    Gap,
}

pub struct ViewportWidget {
    rows: Vec<RenderedRow>,
    scroll_from_bottom: usize,
}

impl Widget for ViewportWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.rows.is_empty() {
            return;
        }
        let view_height = area.height as usize;
        let total = self.rows.len();
        let max_scroll = total.saturating_sub(view_height);
        let scroll = self.scroll_from_bottom.min(max_scroll);
        let end = total.saturating_sub(scroll);
        let start = end.saturating_sub(view_height);

        for (row, item) in self.rows.iter().skip(start).take(view_height).enumerate() {
            let y = area.y + row as u16;
            match item {
                RenderedRow::Line { role, first, line } => {
                    paint_message(buf, area, y, *role, *first, line);
                }
                RenderedRow::Gap => {}
            }
        }
    }
}

fn prefix(role: Role, first: bool) -> &'static str {
    match (role, first) {
        (Role::User, true) => USER_PREFIX,
        (Role::Assistant, true) => ASSISTANT_PREFIX,
        (Role::User, false) | (Role::Assistant, false) => "  ",
    }
}

fn paint_message(buf: &mut Buffer, area: Rect, y: u16, role: Role, first: bool, line: &Line<'_>) {
    let prefix = prefix(role, first);
    let prefix_width = Line::from(prefix).width();
    let mut x = area.x;

    if role == Role::User {
        buf.set_style(
            Rect {
                x: area.x,
                y,
                width: area.width,
                height: 1,
            },
            Style::default().bg(USER_BAR),
        );
        render_clipped(
            buf,
            x,
            y,
            prefix,
            area.width as usize,
            Style::default().fg(USER_FG).bg(USER_BAR),
        );
    } else {
        render_clipped(
            buf,
            x,
            y,
            prefix,
            area.width as usize,
            Style::default().fg(BULLET),
        );
    }

    x = x.saturating_add(prefix_width as u16);
    for span in &line.spans {
        if x >= area.x + area.width {
            break;
        }
        let remaining = (area.x + area.width).saturating_sub(x) as usize;
        let style = match role {
            Role::User => span.style.fg(USER_FG).bg(USER_BAR),
            Role::Assistant => span.style,
        };
        render_clipped(buf, x, y, span.content.as_ref(), remaining, style);
        x = x.saturating_add(Line::from(span.content.as_ref()).width() as u16);
    }
}

#[cfg(test)]
mod tests {
    use super::{Role, USER_BAR, ViewportState};
    use crate::frontend::chat::cache::MarkdownCache;
    use ratatui::{Terminal, backend::TestBackend, style::Color};

    fn screen(terminal: &Terminal<TestBackend>, width: u16, height: u16) -> String {
        let buffer = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..height {
            for x in 0..width {
                text.push_str(buffer.cell((x, y)).unwrap().symbol());
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn new_messages_pin_to_the_bottom() {
        let mut viewport = ViewportState::default();
        viewport.push(Role::User, "one".into());
        viewport.scroll_from_bottom = 4;
        viewport.push(Role::User, "two".into());
        assert_eq!(viewport.scroll_from_bottom, 0);
        assert_eq!(viewport.messages.len(), 2);
    }

    #[test]
    fn scroll_up_clamps_to_hidden_lines() {
        let mut viewport = ViewportState::default();
        viewport.push(Role::User, "alpha".into());
        viewport.push(Role::Assistant, "beta".into());
        let mut cache = MarkdownCache::default();
        let height = viewport.content_height(&mut cache, 20);
        viewport.scroll_up(100, height, 2);
        assert_eq!(viewport.scroll_from_bottom, height.saturating_sub(2));
        viewport.scroll_down(100);
        assert_eq!(viewport.scroll_from_bottom, 0);
    }

    #[test]
    fn user_messages_are_a_highlighted_prompt_bar() {
        let mut viewport = ViewportState::default();
        viewport.push(Role::User, "**ready**".into());
        let mut cache = MarkdownCache::default();
        let mut terminal = Terminal::new(TestBackend::new(24, 3)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(viewport.widget(&mut cache, 22), frame.area());
            })
            .unwrap();

        let rendered = screen(&terminal, 24, 3);
        assert!(rendered.contains("> ready"));
        assert!(!rendered.contains("you"));
        assert!(!rendered.contains("**"));

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((0, 0)).unwrap().bg, USER_BAR);
        assert_eq!(buffer.cell((23, 0)).unwrap().bg, USER_BAR);
        assert_eq!(buffer.cell((0, 0)).unwrap().symbol(), ">");
    }

    #[test]
    fn assistant_messages_use_a_bullet_on_the_open_background() {
        let mut viewport = ViewportState::default();
        viewport.push(Role::Assistant, "I'll run it".into());
        let mut cache = MarkdownCache::default();
        let mut terminal = Terminal::new(TestBackend::new(24, 2)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(viewport.widget(&mut cache, 22), frame.area());
            })
            .unwrap();

        let rendered = screen(&terminal, 24, 2);
        assert!(rendered.contains("● I'll run it") || rendered.contains("I'll run it"));
        assert!(rendered.contains('●'));
        assert!(!rendered.contains("raid"));

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((0, 0)).unwrap().bg, Color::Reset);
        assert_eq!(buffer.cell((0, 0)).unwrap().symbol(), "●");
    }

    #[test]
    fn short_history_starts_at_the_top_of_the_viewport() {
        let mut viewport = ViewportState::default();
        viewport.push(Role::User, "hi".into());
        let mut cache = MarkdownCache::default();
        let mut terminal = Terminal::new(TestBackend::new(20, 8)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(viewport.widget(&mut cache, 20), frame.area());
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((0, 0)).unwrap().symbol(), ">");
        assert_eq!(buffer.cell((2, 0)).unwrap().symbol(), "h");
        assert_eq!(buffer.cell((0, 7)).unwrap().symbol(), " ");
    }
}
