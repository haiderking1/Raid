mod keys;
mod palette;

#[cfg(test)]
mod tests;

use super::wrap::visual_lines_for_cursor;

const MAX_VISIBLE_LINES: usize = 8;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ComposerState {
    text: String,
    cursor: usize,
    vertical_column: Option<usize>,
    palette_selected: usize,
    palette_dismissed: bool,
}

impl ComposerState {
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
        self.on_text_changed();
    }

    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    pub(crate) fn cursor(&self) -> usize {
        self.cursor
    }
}

#[cfg(test)]
impl ComposerState {
    fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> super::action::ComposerAction {
        self.handle_key_with_width(key, usize::MAX)
    }
}
