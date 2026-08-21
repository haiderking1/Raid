use ratatui::layout::Rect;

const PROMPT_COLUMNS: u16 = 2;
const CURSOR_RESERVE: u16 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputRowLayout {
    pub prompt_x: u16,
    pub text_x: u16,
    pub wrap_width: usize,
    pub render_width: usize,
}

pub fn composer_input_layout(inner: Rect) -> InputRowLayout {
    InputRowLayout {
        prompt_x: inner.x,
        text_x: inner.x.saturating_add(PROMPT_COLUMNS),
        wrap_width: inner
            .width
            .saturating_sub(PROMPT_COLUMNS + CURSOR_RESERVE) as usize,
        render_width: inner.width.saturating_sub(PROMPT_COLUMNS) as usize,
    }
}

pub fn padded_input_layout(area: Rect) -> InputRowLayout {
    const ROW_PADDING: u16 = 2;
    const ROW_RIGHT_PADDING: u16 = 2;
    InputRowLayout {
        prompt_x: area.x.saturating_add(ROW_PADDING),
        text_x: area.x.saturating_add(ROW_PADDING + PROMPT_COLUMNS),
        wrap_width: area
            .width
            .saturating_sub(ROW_PADDING + PROMPT_COLUMNS + ROW_RIGHT_PADDING + CURSOR_RESERVE)
            as usize,
        render_width: area
            .width
            .saturating_sub(ROW_PADDING + PROMPT_COLUMNS + ROW_RIGHT_PADDING) as usize,
    }
}
