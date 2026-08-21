use ratatui::{buffer::Buffer, style::Style, text::Line};
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
