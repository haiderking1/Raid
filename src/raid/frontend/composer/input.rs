use super::metrics::InputRowLayout;
use super::state::ComposerState;
use super::wrap::ComposerLayout;
use crate::frontend::clip::render_clipped_with_cursor;
use ratatui::{buffer::Buffer, style::Style};

pub fn paint_input_editor(
    buf: &mut Buffer,
    row_layout: InputRowLayout,
    input: &ComposerState,
    origin_y: u16,
    visible_height: usize,
    text_style: Style,
    prompt_style: Style,
) {
    if row_layout.wrap_width == 0 || visible_height == 0 {
        return;
    }

    let layout = ComposerLayout::new(
        input.text(),
        input.cursor().min(input.text().len()),
        row_layout.wrap_width,
        visible_height,
    );

    for (row_index, line) in layout
        .lines
        .iter()
        .skip(layout.scroll_top)
        .take(visible_height)
        .enumerate()
    {
        let line_index = layout.scroll_top + row_index;
        let slice = &input.text()[line.start..line.end];
        let cursor_offset = (line_index == layout.cursor_line).then(|| {
            input.cursor().min(line.end).saturating_sub(line.start)
        });
        render_clipped_with_cursor(
            buf,
            row_layout.text_x,
            origin_y + row_index as u16,
            slice,
            cursor_offset,
            row_layout.render_width,
            text_style,
        );
    }
    buf.set_string(row_layout.prompt_x, origin_y, ">", prompt_style);
}
