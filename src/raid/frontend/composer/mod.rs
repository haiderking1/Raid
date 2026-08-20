use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    text::Line,
    widgets::{Block, BorderType, Borders, Widget},
};
use unicode_segmentation::UnicodeSegmentation;

pub mod slash_commands;

const MAX_VISIBLE_LINES: usize = 8;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ComposerState {
    text: String,
    cursor: usize,
    vertical_column: Option<usize>,
}

impl ComposerState {
    pub fn handle_key_with_width(&mut self, key: KeyEvent, content_width: usize) -> ComposerAction {
        if key.kind != KeyEventKind::Press {
            return ComposerAction::None;
        }

        if key.code == KeyCode::Esc
            || (key.code == KeyCode::Char('c')
                && key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT))
        {
            return ComposerAction::Quit;
        }

        if !matches!(key.code, KeyCode::Up | KeyCode::Down) {
            self.vertical_column = None;
        }

        match key.code {
            KeyCode::Char(character) if accepts_character(character, key.modifiers) => {
                self.insert_character(character);
                ComposerAction::None
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let previous = previous_grapheme_boundary(&self.text, self.cursor);
                    self.text.drain(previous..self.cursor);
                    self.cursor = previous;
                }
                ComposerAction::None
            }
            KeyCode::Delete => {
                if self.cursor < self.text.len() {
                    let next = next_grapheme_boundary(&self.text, self.cursor);
                    self.text.drain(self.cursor..next);
                }
                ComposerAction::None
            }
            KeyCode::Left => {
                self.cursor = previous_grapheme_boundary(&self.text, self.cursor);
                ComposerAction::None
            }
            KeyCode::Right => {
                self.cursor = next_grapheme_boundary(&self.text, self.cursor);
                ComposerAction::None
            }
            KeyCode::Up => {
                let column = self.vertical_column.unwrap_or_else(|| {
                    cursor_visual_column(&self.text, self.cursor, content_width)
                });
                self.cursor = move_vertical(&self.text, self.cursor, content_width, true, column);
                self.vertical_column = Some(column);
                ComposerAction::None
            }
            KeyCode::Down => {
                let column = self.vertical_column.unwrap_or_else(|| {
                    cursor_visual_column(&self.text, self.cursor, content_width)
                });
                self.cursor = move_vertical(&self.text, self.cursor, content_width, false, column);
                self.vertical_column = Some(column);
                ComposerAction::None
            }
            KeyCode::Home => {
                self.cursor = if key.modifiers.contains(KeyModifiers::CONTROL) {
                    0
                } else {
                    visual_line_start(&self.text, self.cursor, content_width)
                };
                ComposerAction::None
            }
            KeyCode::End => {
                self.cursor = if key.modifiers.contains(KeyModifiers::CONTROL) {
                    self.text.len()
                } else {
                    visual_line_end(&self.text, self.cursor, content_width)
                };
                ComposerAction::None
            }
            KeyCode::Enter => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    self.insert_character('\n');
                    ComposerAction::None
                } else if self.text.trim().is_empty() {
                    ComposerAction::None
                } else {
                    let submitted = std::mem::take(&mut self.text);
                    self.cursor = 0;
                    ComposerAction::Submit(submitted)
                }
            }
            _ => ComposerAction::None,
        }
    }

    pub fn insert_paste(&mut self, pasted: &str) {
        let mut characters = pasted.chars().peekable();
        while let Some(character) = characters.next() {
            match character {
                '\r' => {
                    if characters.peek() == Some(&'\n') {
                        characters.next();
                    }
                    self.insert_character('\n');
                }
                '\n' | '\u{2028}' | '\u{2029}' => self.insert_character('\n'),
                '\t' => {
                    for _ in 0..4 {
                        self.insert_character(' ');
                    }
                }
                character if character.is_control() => {}
                character => self.insert_character(character),
            }
        }
    }

    pub fn desired_height(&self, content_width: usize, max_height: u16) -> u16 {
        if max_height < 3 {
            return max_height;
        }
        let max_lines = usize::from(max_height.saturating_sub(2)).clamp(1, MAX_VISIBLE_LINES);
        let line_count = visual_lines_for_cursor(&self.text, self.cursor, content_width).len();
        (line_count.min(max_lines) + 2) as u16
    }

    fn insert_character(&mut self, character: char) {
        self.text.insert(self.cursor, character);
        self.cursor += character.len_utf8();
        self.vertical_column = None;
    }
}

#[cfg(test)]
impl ComposerState {
    fn handle_key(&mut self, key: KeyEvent) -> ComposerAction {
        self.handle_key_with_width(key, usize::MAX)
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ComposerAction {
    None,
    Submit(String),
    Quit,
}

pub struct ComposerWidget<'a> {
    state: &'a ComposerState,
}

impl<'a> ComposerWidget<'a> {
    pub fn new(state: &'a ComposerState) -> Self {
        Self { state }
    }

    pub fn cursor_position(area: Rect, state: &ComposerState) -> Option<(u16, u16)> {
        let inner = Self::block().inner(area);
        let content_width = inner.width.saturating_sub(2) as usize;
        if content_width == 0 || inner.height == 0 {
            return None;
        }

        let text_x = inner.x.saturating_add(2);
        let layout = ComposerLayout::new(
            &state.text,
            state.cursor.min(state.text.len()),
            content_width,
            inner.height as usize,
        );
        let cursor_row = layout.cursor_line.saturating_sub(layout.scroll_top);
        let cursor_x = text_x.saturating_add(
            layout
                .cursor_width
                .min(content_width.saturating_sub(1))
                .try_into()
                .unwrap_or(u16::MAX),
        );
        Some((cursor_x, inner.y + cursor_row as u16))
    }

    fn block() -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Rgb(96, 96, 96)))
    }
}

impl Widget for ComposerWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let block = Self::block();
        let inner = block.inner(area);
        block.render(area, buf);

        let content_width = inner.width.saturating_sub(2) as usize;
        if content_width == 0 || inner.height == 0 {
            return;
        }

        let text_x = inner.x.saturating_add(2);
        let prompt_style = Style::default().fg(Color::Rgb(190, 190, 190));
        let text_style = Style::default().fg(Color::White);
        let layout = ComposerLayout::new(
            &self.state.text,
            self.state.cursor.min(self.state.text.len()),
            content_width,
            inner.height as usize,
        );

        for (row, line) in layout
            .lines
            .iter()
            .skip(layout.scroll_top)
            .take(inner.height as usize)
            .enumerate()
        {
            render_text_line(
                buf,
                text_x,
                inner.y + row as u16,
                &self.state.text[line.start..line.end],
                content_width,
                text_style,
            );
        }
        buf.set_string(inner.x, inner.y, ">", prompt_style);
    }
}

fn render_text_line(buf: &mut Buffer, x: u16, y: u16, text: &str, width: usize, style: Style) {
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

fn accepts_character(character: char, modifiers: KeyModifiers) -> bool {
    let alt_gr = modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT);
    !character.is_control()
        && !matches!(character, '\u{2028}' | '\u{2029}')
        && (!modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) || alt_gr)
}

fn previous_grapheme_boundary(text: &str, cursor: usize) -> usize {
    text[..cursor]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_grapheme_boundary(text: &str, cursor: usize) -> usize {
    text[cursor..]
        .graphemes(true)
        .next()
        .map_or(cursor, |grapheme| cursor + grapheme.len())
}

fn cursor_visual_column(text: &str, cursor: usize, width: usize) -> usize {
    let lines = visual_lines_for_cursor(text, cursor, width);
    let line = lines[cursor_line(&lines, cursor)];
    Line::from(&text[line.start..cursor.min(line.end)]).width()
}

fn visual_line_start(text: &str, cursor: usize, width: usize) -> usize {
    let lines = visual_lines_for_cursor(text, cursor, width);
    lines[cursor_line(&lines, cursor)].start
}

fn visual_line_end(text: &str, cursor: usize, width: usize) -> usize {
    let lines = visual_lines_for_cursor(text, cursor, width);
    lines[cursor_line(&lines, cursor)].end
}

fn move_vertical(text: &str, cursor: usize, width: usize, up: bool, column: usize) -> usize {
    let lines = visual_lines_for_cursor(text, cursor, width);
    let current_line = cursor_line(&lines, cursor);
    let target_line = if up {
        current_line.checked_sub(1)
    } else {
        current_line
            .checked_add(1)
            .filter(|&line| line < lines.len())
    };

    target_line.map_or(cursor, |line| {
        position_in_line(text, lines[line].start, lines[line].end, column)
    })
}

fn position_in_line(text: &str, start: usize, end: usize, column: usize) -> usize {
    let mut position = start;
    let mut width = 0;
    for (offset, grapheme) in text[start..end].grapheme_indices(true) {
        let grapheme_width = Line::from(grapheme).width();
        if width + grapheme_width > column {
            break;
        }
        width += grapheme_width;
        position = start + offset + grapheme.len();
    }
    position
}

#[derive(Debug, Clone, Copy)]
struct VisualLine {
    start: usize,
    end: usize,
    width: usize,
}

fn visual_lines(text: &str, width: usize) -> Vec<VisualLine> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut logical_start = 0;

    for logical_line in text.split('\n') {
        let logical_end = logical_start + logical_line.len();
        if logical_line.is_empty() {
            lines.push(VisualLine {
                start: logical_start,
                end: logical_end,
                width: 0,
            });
        } else {
            let mut segment_start = logical_start;
            let mut segment_width = 0;
            for (offset, grapheme) in logical_line.grapheme_indices(true) {
                let grapheme_width = Line::from(grapheme).width();
                if segment_width > 0 && segment_width + grapheme_width > width {
                    lines.push(VisualLine {
                        start: segment_start,
                        end: logical_start + offset,
                        width: segment_width,
                    });
                    segment_start = logical_start + offset;
                    segment_width = 0;
                }
                segment_width += grapheme_width;
            }
            lines.push(VisualLine {
                start: segment_start,
                end: logical_end,
                width: segment_width,
            });
        }
        logical_start = logical_end + '\n'.len_utf8();
    }

    lines
}

fn visual_lines_for_cursor(text: &str, cursor: usize, width: usize) -> Vec<VisualLine> {
    let mut lines = visual_lines(text, width);
    let width = width.max(1);
    let cursor = cursor.min(text.len());
    let current_line = cursor_line(&lines, cursor);
    let line = lines[current_line];
    let is_soft_wrap = lines
        .get(current_line + 1)
        .is_some_and(|next| next.start == cursor);
    if cursor == line.end
        && line.width >= width
        && !is_soft_wrap
        && !text[cursor..].starts_with('\n')
    {
        lines.insert(
            current_line + 1,
            VisualLine {
                start: cursor,
                end: cursor,
                width: 0,
            },
        );
    }
    lines
}

struct ComposerLayout {
    lines: Vec<VisualLine>,
    cursor_line: usize,
    cursor_width: usize,
    scroll_top: usize,
}

impl ComposerLayout {
    fn new(text: &str, cursor: usize, content_width: usize, visible_height: usize) -> Self {
        let lines = visual_lines_for_cursor(text, cursor, content_width);
        let cursor_line = cursor_line(&lines, cursor);
        let visible_height = visible_height.max(1).min(lines.len());
        let scroll_top = cursor_line.saturating_sub(visible_height - 1);
        let line = lines[cursor_line];
        let cursor_width = Line::from(&text[line.start..cursor.min(line.end)]).width();

        Self {
            lines,
            cursor_line,
            cursor_width,
            scroll_top,
        }
    }
}

fn cursor_line(lines: &[VisualLine], cursor: usize) -> usize {
    for (index, line) in lines.iter().enumerate() {
        if cursor < line.end {
            return index;
        }
        if cursor == line.end {
            let is_soft_wrap = lines
                .get(index + 1)
                .is_some_and(|next| next.start == cursor);
            if !is_soft_wrap {
                return index;
            }
        }
    }
    lines.len().saturating_sub(1)
}

#[cfg(test)]
mod tests {
    use super::{ComposerAction, ComposerState, ComposerWidget, visual_lines};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn edits_text_and_tracks_utf8_cursor() {
        let mut composer = ComposerState::default();

        composer.handle_key(key(KeyCode::Char('h')));
        composer.handle_key(key(KeyCode::Char('i')));
        composer.handle_key(key(KeyCode::Left));
        composer.handle_key(key(KeyCode::Char('é')));

        assert_eq!(composer.text, "héi");
        assert_eq!(composer.cursor, "hé".len());
    }

    #[test]
    fn backspace_removes_the_previous_character() {
        let mut composer = ComposerState::default();
        composer.handle_key(key(KeyCode::Char('a')));
        composer.handle_key(key(KeyCode::Char('界')));
        composer.handle_key(key(KeyCode::Backspace));

        assert_eq!(composer.text, "a");
        assert_eq!(composer.cursor, 1);
    }

    #[test]
    fn editing_stays_on_grapheme_boundaries() {
        let mut composer = ComposerState::default();
        composer.insert_paste("e\u{301}");
        composer.handle_key(key(KeyCode::Backspace));

        assert_eq!(composer.text, "");
        assert_eq!(composer.cursor, 0);
    }

    #[test]
    fn paste_normalizes_lines_and_drops_control_characters() {
        let mut composer = ComposerState::default();
        composer.insert_paste("one\r\n\t two\u{2028}three\u{2029}four\u{0007}");

        assert_eq!(composer.text, "one\n     two\nthree\nfour");
    }

    #[test]
    fn shift_enter_inserts_a_newline_without_submitting() {
        let mut composer = ComposerState::default();
        composer.handle_key(key(KeyCode::Char('a')));

        assert_eq!(
            composer.handle_key(modified_key(KeyCode::Enter, KeyModifiers::SHIFT)),
            ComposerAction::None
        );
        composer.handle_key(key(KeyCode::Char('b')));

        assert_eq!(composer.text, "a\nb");
        assert_eq!(composer.cursor, "a\nb".len());
    }

    #[test]
    fn vertical_navigation_preserves_the_text_column() {
        let mut composer = ComposerState::default();
        composer.insert_paste("first\nsecond");
        composer.handle_key(key(KeyCode::Home));
        composer.handle_key(key(KeyCode::Up));

        assert_eq!(composer.cursor, 0);

        composer.handle_key(key(KeyCode::Down));
        composer.handle_key(key(KeyCode::End));
        assert_eq!(composer.cursor, composer.text.len());
    }

    #[test]
    fn full_width_text_before_a_newline_does_not_add_a_caret_row() {
        let mut composer = ComposerState::default();
        composer.insert_paste("abc\ndef");
        composer.cursor = "abc".len();

        assert_eq!(composer.desired_height(3, 20), 4);
    }

    #[test]
    fn vertical_navigation_follows_soft_wrapped_lines() {
        let mut composer = ComposerState::default();
        composer.insert_paste("abcdef");

        composer.handle_key_with_width(modified_key(KeyCode::Home, KeyModifiers::CONTROL), 3);
        composer.handle_key_with_width(key(KeyCode::Down), 3);
        assert_eq!(composer.cursor, 3);

        composer.handle_key_with_width(key(KeyCode::Up), 3);
        assert_eq!(composer.cursor, 0);

        composer.handle_key_with_width(key(KeyCode::End), 3);
        assert_eq!(composer.cursor, 3);
    }

    #[test]
    fn desired_height_respects_tiny_maximums() {
        let composer = ComposerState::default();

        assert_eq!(composer.desired_height(8, 0), 0);
        assert_eq!(composer.desired_height(8, 1), 1);
        assert_eq!(composer.desired_height(8, 2), 2);
    }

    #[test]
    fn alt_gr_character_is_inserted() {
        let mut composer = ComposerState::default();
        let alt_gr = KeyEvent::new(
            KeyCode::Char('@'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
        );

        composer.handle_key(alt_gr);

        assert_eq!(composer.text, "@");
    }

    #[test]
    fn control_key_characters_are_not_inserted() {
        let mut composer = ComposerState::default();

        composer.handle_key(KeyEvent::new(KeyCode::Char('\u{0007}'), KeyModifiers::NONE));

        assert_eq!(composer.text, "");
    }

    #[test]
    fn wrapping_keeps_combining_graphemes_together() {
        let text = "e\u{301}x";
        let lines = visual_lines(text, 1);

        assert_eq!(lines.len(), 2);
        assert_eq!(&text[lines[0].start..lines[0].end], "e\u{301}");
        assert_eq!(&text[lines[1].start..lines[1].end], "x");
    }

    #[test]
    fn renders_the_prompt_and_keeps_text_inside_the_border() {
        let mut composer = ComposerState::default();
        composer.insert_paste("abcdefghijk");
        let mut terminal = Terminal::new(TestBackend::new(12, 3)).unwrap();

        terminal
            .draw(|frame| {
                frame.render_widget(ComposerWidget::new(&composer), frame.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((0, 0)).unwrap().symbol(), "╭");
        assert_eq!(buffer.cell((1, 1)).unwrap().symbol(), ">");
        assert_eq!(buffer.cell((11, 1)).unwrap().symbol(), "│");
        assert_eq!(
            ComposerWidget::cursor_position(Rect::new(0, 0, 12, 3), &composer),
            Some((6, 1))
        );
    }

    #[test]
    fn creates_a_caret_row_when_text_fills_the_line() {
        let mut composer = ComposerState::default();
        composer.insert_paste("12345678");
        assert_eq!(composer.desired_height(8, 20), 4);

        let mut terminal = Terminal::new(TestBackend::new(12, 4)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(ComposerWidget::new(&composer), frame.area());
            })
            .unwrap();

        assert_eq!(
            ComposerWidget::cursor_position(Rect::new(0, 0, 12, 4), &composer),
            Some((3, 2))
        );
    }

    #[test]
    fn renders_a_placeholder_for_a_wide_glyph_on_a_one_cell_line() {
        let mut composer = ComposerState::default();
        composer.insert_paste("界");
        let mut terminal = Terminal::new(TestBackend::new(5, 4)).unwrap();

        terminal
            .draw(|frame| {
                frame.render_widget(ComposerWidget::new(&composer), frame.area());
            })
            .unwrap();

        assert_eq!(
            terminal.backend().buffer().cell((3, 1)).unwrap().symbol(),
            "…"
        );
    }

    #[test]
    fn multiline_composer_grows_and_positions_the_cursor_on_the_active_line() {
        let mut composer = ComposerState::default();
        composer.insert_paste("one\ntwo");
        assert_eq!(composer.desired_height(8, 20), 4);

        let mut terminal = Terminal::new(TestBackend::new(12, 4)).unwrap();
        terminal
            .draw(|frame| {
                frame.render_widget(ComposerWidget::new(&composer), frame.area());
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer.cell((3, 1)).unwrap().symbol(), "o");
        assert_eq!(buffer.cell((3, 2)).unwrap().symbol(), "t");
        assert_eq!(
            ComposerWidget::cursor_position(Rect::new(0, 0, 12, 4), &composer),
            Some((6, 2))
        );
    }

    #[test]
    fn enter_submits_and_resets_the_composer() {
        let mut composer = ComposerState::default();
        composer.handle_key(key(KeyCode::Char('r')));
        composer.handle_key(key(KeyCode::Char('u')));
        composer.handle_key(key(KeyCode::Char('n')));

        assert_eq!(
            composer.handle_key(key(KeyCode::Enter)),
            ComposerAction::Submit("run".to_owned())
        );
        assert_eq!(composer, ComposerState::default());
    }

    #[test]
    fn escape_quits_without_submitting_text() {
        let mut composer = ComposerState::default();
        composer.handle_key(key(KeyCode::Char('x')));

        assert_eq!(composer.handle_key(key(KeyCode::Esc)), ComposerAction::Quit);
        assert_eq!(composer.text, "x");
    }
}
