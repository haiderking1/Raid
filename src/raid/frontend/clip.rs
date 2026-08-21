use ratatui::{buffer::Buffer, style::Modifier, style::Style, text::Line};
use unicode_segmentation::UnicodeSegmentation;

pub fn render_clipped(buf: &mut Buffer, x: u16, y: u16, text: &str, width: usize, style: Style) {
    if width == 0 {
        return;
    }
    let mut visible = String::new();
    let mut used_width = 0;
    for grapheme in text.graphemes(true) {
        let grapheme_width = Line::from(grapheme).width();
        if used_width + grapheme_width > width {
            if used_width == 0 {
                visible.push('…');
            }
            break;
        }
        visible.push_str(grapheme);
        used_width += grapheme_width;
    }
    buf.set_stringn(x, y, &visible, width, style);
}

pub fn render_clipped_with_cursor(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    text: &str,
    cursor_offset: Option<usize>,
    width: usize,
    style: Style,
) {
    let Some(cursor_offset) = cursor_offset else {
        render_clipped(buf, x, y, text, width, style);
        return;
    };

    let cursor_offset = cursor_offset.min(text.len());
    let before = &text[..cursor_offset];
    let after = &text[cursor_offset..];
    let before_width = Line::from(before).width();
    render_clipped(buf, x, y, before, width.min(before_width.max(1)), style);

    let cursor_x = x.saturating_add(before_width as u16);
    let remaining = width.saturating_sub(before_width);
    if remaining == 0 {
        return;
    }

    if after.is_empty() {
        render_clipped(
            buf,
            cursor_x,
            y,
            " ",
            1,
            style.add_modifier(Modifier::REVERSED),
        );
        return;
    }

    let first = after.graphemes(true).next().unwrap_or("");
    let first_width = Line::from(first).width();
    render_clipped(
        buf,
        cursor_x,
        y,
        first,
        first_width.min(remaining),
        style.add_modifier(Modifier::REVERSED),
    );
    let rest_x = cursor_x.saturating_add(first_width as u16);
    let rest_width = remaining.saturating_sub(first_width);
    if rest_width > 0 && first.len() < after.len() {
        render_clipped(buf, rest_x, y, &after[first.len()..], rest_width, style);
    }
}
