use crate::frontend::clip::render_clipped;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

const HEADER: Color = Color::Rgb(80, 196, 184);

pub struct ConnectHeaderWidget<'a> {
    text: &'a str,
}

impl<'a> ConnectHeaderWidget<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }
}

impl Widget for ConnectHeaderWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        render_clipped(
            buf,
            area.x,
            area.y,
            self.text,
            area.width as usize,
            Style::default().fg(HEADER),
        );
    }
}

pub struct ConnectFooterWidget<'a> {
    text: &'a str,
}

impl<'a> ConnectFooterWidget<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }
}

impl Widget for ConnectFooterWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        render_clipped(
            buf,
            area.x,
            area.y,
            self.text,
            area.width as usize,
            Style::default().fg(Color::Rgb(96, 96, 96)),
        );
    }
}

pub struct ConnectLabelWidget<'a> {
    text: &'a str,
}

impl<'a> ConnectLabelWidget<'a> {
    pub fn new(text: &'a str) -> Self {
        Self { text }
    }
}

impl Widget for ConnectLabelWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        render_clipped(
            buf,
            area.x.saturating_add(2),
            area.y,
            self.text,
            area.width.saturating_sub(2) as usize,
            Style::default().fg(Color::Rgb(228, 228, 228)),
        );
    }
}
