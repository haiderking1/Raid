use crate::frontend::clip::render_clipped;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, BorderType, Borders, Widget},
};

pub struct ConnectPromptWidget<'a> {
    prompt: &'a str,
    masked: &'a str,
}

impl<'a> ConnectPromptWidget<'a> {
    pub fn new(prompt: &'a str, masked: &'a str) -> Self {
        Self { prompt, masked }
    }

    pub fn cursor_position(area: Rect, _prompt: &str, masked: &str) -> Option<(u16, u16)> {
        let inner = block().inner(area);
        if inner.width < 4 || inner.height == 0 {
            return None;
        }
        let text_x = inner.x.saturating_add(2);
        let cursor_x = text_x.saturating_add(masked.len() as u16);
        Some((cursor_x.min(inner.x + inner.width - 1), inner.y))
    }
}

fn block() -> Block<'static> {
    Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Rgb(96, 96, 96)))
}

impl Widget for ConnectPromptWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = block();
        let inner = block.inner(area);
        block.render(area, buf);
        if inner.width == 0 || inner.height == 0 {
            return;
        }

        let prompt_style = Style::default().fg(Color::Rgb(190, 190, 190));
        let secret_style = Style::default().fg(Color::White);
        let text_x = inner.x.saturating_add(2);
        let available = inner.width.saturating_sub(2) as usize;
        render_clipped(buf, inner.x, inner.y, ">", 1, prompt_style);
        render_clipped(buf, text_x, inner.y, self.prompt, available, prompt_style);
        let prompt_width = self.prompt.len().min(available);
        render_clipped(
            buf,
            text_x.saturating_add(prompt_width as u16),
            inner.y,
            self.masked,
            available.saturating_sub(prompt_width),
            secret_style,
        );
    }
}
