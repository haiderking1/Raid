use std::path::Path;

use crate::backend::session::SessionSummary;
use crate::frontend::clip::render_clipped;
use crate::frontend::composer::{paint_input_editor, padded_input_layout, ComposerState};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

const MAX_VISIBLE_SESSIONS: usize = 8;
const FIXED_ROWS: u16 = 5;
const BORDER: Color = Color::Rgb(72, 92, 128);
const TEXT: Color = Color::Rgb(228, 228, 228);
const DIM: Color = Color::Rgb(130, 148, 150);
const MUTED: Color = Color::Rgb(96, 96, 96);
const SELECTED: Color = Color::Rgb(80, 196, 184);

pub struct SessionPaletteWidget<'a> {
    search: &'a ComposerState,
    sessions: &'a [SessionSummary],
    filtered: &'a [usize],
    selected: usize,
    status: &'a str,
    current: Option<&'a Path>,
}

impl<'a> SessionPaletteWidget<'a> {
    pub fn new(
        search: &'a ComposerState,
        sessions: &'a [SessionSummary],
        filtered: &'a [usize],
        selected: usize,
        status: &'a str,
        current: Option<&'a Path>,
    ) -> Self {
        Self {
            search,
            sessions,
            filtered,
            selected: selected.min(filtered.len().saturating_sub(1)),
            status,
            current,
        }
    }
}

impl Widget for SessionPaletteWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        paint_border(buf, area, area.y);
        paint_border(buf, area, area.bottom().saturating_sub(1));
        let input_y = area.y.saturating_add(1);
        let input_layout = padded_input_layout(area);
        if input_layout.wrap_width > 0 {
            paint_input_editor(
                buf,
                input_layout,
                self.search,
                input_y,
                1,
                Style::default().fg(TEXT),
                Style::default().fg(TEXT),
            );
        }

        let mut y = input_y.saturating_add(2);
        if self.filtered.is_empty() {
            paint_text(buf, area, y, "no saved sessions", Style::default().fg(DIM));
            y = y.saturating_add(1);
        } else {
            let capacity = area.height.saturating_sub(FIXED_ROWS).max(1) as usize;
            let visible = self
                .filtered
                .len()
                .min(capacity)
                .clamp(1, MAX_VISIBLE_SESSIONS);
            let start = scroll_top(self.selected, self.filtered.len(), visible);
            for (row, session_index) in self.filtered.iter().skip(start).take(visible).enumerate() {
                let Some(session) = self.sessions.get(*session_index) else {
                    continue;
                };
                let selected = start + row == self.selected;
                let marker = if selected { "→" } else { " " };
                let lock = if self.current == Some(session.path.as_path()) {
                    "  current"
                } else if session.locked {
                    "  open"
                } else {
                    ""
                };
                let text = format!(
                    "{marker} {}  {} messages{lock}",
                    session.title, session.message_count
                );
                paint_text(
                    buf,
                    area,
                    y,
                    &text,
                    Style::default().fg(if selected { SELECTED } else { DIM }),
                );
                y = y.saturating_add(1);
            }
        }

        let footer = if self.status.is_empty() {
            "enter resume  ctrl+d trash  esc close"
        } else {
            self.status
        };
        paint_text(buf, area, y, footer, Style::default().fg(MUTED));
    }
}

fn paint_border(buf: &mut Buffer, area: Rect, y: u16) {
    render_clipped(
        buf,
        area.x,
        y,
        &"─".repeat(area.width as usize),
        area.width as usize,
        Style::default().fg(BORDER),
    );
}

fn paint_text(buf: &mut Buffer, area: Rect, y: u16, text: &str, style: Style) {
    render_clipped(
        buf,
        area.x.saturating_add(2),
        y,
        text,
        area.width.saturating_sub(2) as usize,
        style,
    );
}

fn scroll_top(selected: usize, total: usize, visible: usize) -> usize {
    if total <= visible {
        0
    } else {
        selected.saturating_sub(visible - 1).min(total - visible)
    }
}

pub fn session_palette_height(count: usize, max_height: u16) -> u16 {
    if max_height < FIXED_ROWS + 1 {
        return 0;
    }
    let rows = count.clamp(1, MAX_VISIBLE_SESSIONS) as u16;
    (FIXED_ROWS + rows).min(max_height)
}

pub fn session_input_wrap_width(panel_width: u16) -> usize {
    padded_input_layout(Rect::new(0, 0, panel_width.max(1), 1))
        .wrap_width
        .max(1)
}
