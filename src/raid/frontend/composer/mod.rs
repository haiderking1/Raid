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

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ComposerState {
    text: String,
    cursor: usize,
}

impl ComposerState {
    pub fn handle_key(&mut self, key: KeyEvent) -> ComposerAction {
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
            KeyCode::Home => {
                self.cursor = 0;
                ComposerAction::None
            }
            KeyCode::End => {
                self.cursor = self.text.len();
                ComposerAction::None
            }
            KeyCode::Enter => {
                if self.text.is_empty() {
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
                    self.insert_character(' ');
                }
                '\n' | '\u{2028}' | '\u{2029}' => self.insert_character(' '),
                character if character.is_control() => {}
                character => self.insert_character(character),
            }
        }
    }

    fn insert_character(&mut self, character: char) {
        self.text.insert(self.cursor, character);
        self.cursor += character.len_utf8();
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
        if inner.width < 2 || inner.height == 0 {
            return None;
        }

        let text_x = inner.x.saturating_add(2);
        let right = inner.x.saturating_add(inner.width);
        let available_width = right.saturating_sub(text_x) as usize;
        if available_width == 0 {
            return None;
        }

        let cursor = state.cursor.min(state.text.len());
        let (start, _) = visible_window(&state.text, cursor, available_width);
        let cursor_width = Line::from(&state.text[start..cursor]).width();
        let cursor_x = text_x.saturating_add(
            cursor_width
                .min(available_width.saturating_sub(1))
                .try_into()
                .unwrap_or(u16::MAX),
        );
        Some((cursor_x, inner.y + inner.height / 2))
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

        if inner.width < 2 || inner.height == 0 {
            return;
        }

        let y = inner.y + inner.height / 2;
        let text_x = inner.x.saturating_add(2);
        let right = inner.x.saturating_add(inner.width);
        let available_width = right.saturating_sub(text_x) as usize;
        if available_width == 0 {
            return;
        }

        let prompt_style = Style::default().fg(Color::Rgb(190, 190, 190));
        let text_style = Style::default().fg(Color::White);
        buf.set_string(inner.x, y, ">", prompt_style);

        let cursor = self.state.cursor.min(self.state.text.len());
        let (start, end) = visible_window(&self.state.text, cursor, available_width);
        buf.set_stringn(
            text_x,
            y,
            &self.state.text[start..end],
            available_width,
            text_style,
        );
    }
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

fn visible_window(text: &str, cursor: usize, width: usize) -> (usize, usize) {
    if width == 0 {
        return (cursor, cursor);
    }

    let mut start = 0;
    let mut cursor_width = Line::from(&text[..cursor]).width();
    while cursor_width >= width && start < cursor {
        let grapheme = text[start..]
            .graphemes(true)
            .next()
            .expect("start is before cursor");
        let next = start + grapheme.len();
        cursor_width = cursor_width.saturating_sub(Line::from(grapheme).width());
        start = next;
    }

    let mut end = start;
    let mut used_width = 0;
    for (offset, grapheme) in text[start..].grapheme_indices(true) {
        let next = start + offset + grapheme.len();
        let grapheme_width = Line::from(grapheme).width();
        if used_width + grapheme_width > width {
            break;
        }
        used_width += grapheme_width;
        end = next;
    }

    (start, end)
}

#[cfg(test)]
mod tests {
    use super::{ComposerAction, ComposerState, ComposerWidget, visible_window};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend, layout::Rect};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
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
        composer.insert_paste("one\r\ntwo\u{2028}three\u{2029}four\u{0007}");

        assert_eq!(composer.text, "one two three four");
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
    fn viewport_keeps_combining_graphemes_together() {
        let text = "e\u{301}x";
        let (start, end) = visible_window(text, text.len(), 2);

        assert_eq!(&text[start..end], "x");
        assert!(text.is_char_boundary(start));
        assert!(text.is_char_boundary(end));
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
            Some((10, 1))
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
