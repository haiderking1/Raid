use super::ComposerState;
use crate::frontend::composer::action::ComposerAction;
use crate::frontend::composer::wrap::{
    cursor_visual_column, move_vertical, next_grapheme_boundary, previous_grapheme_boundary,
    visual_line_end, visual_line_start,
};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

impl ComposerState {
    pub fn handle_key_with_width(&mut self, key: KeyEvent, content_width: usize) -> ComposerAction {
        if key.kind != KeyEventKind::Press {
            return ComposerAction::None;
        }

        if key.code == KeyCode::Char('c')
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && !key.modifiers.contains(KeyModifiers::ALT)
        {
            if self.text.is_empty() {
                return ComposerAction::Quit;
            }
            *self = Self::default();
            return ComposerAction::None;
        }

        if self.palette_visible()
            && let Some(action) = self.handle_palette_key(key)
        {
            return action;
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
                    self.on_text_changed();
                }
                ComposerAction::None
            }
            KeyCode::Delete => {
                if self.cursor < self.text.len() {
                    let next = next_grapheme_boundary(&self.text, self.cursor);
                    self.text.drain(self.cursor..next);
                    self.on_text_changed();
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
                    *self = Self::default();
                    ComposerAction::Submit(submitted)
                }
            }
            _ => ComposerAction::None,
        }
    }

    fn handle_palette_key(&mut self, key: KeyEvent) -> Option<ComposerAction> {
        match key.code {
            KeyCode::Esc => {
                self.palette_dismissed = true;
                Some(ComposerAction::None)
            }
            KeyCode::Up => {
                self.move_palette(-1);
                Some(ComposerAction::None)
            }
            KeyCode::Down => {
                self.move_palette(1);
                Some(ComposerAction::None)
            }
            KeyCode::Tab if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.complete_selected_command();
                Some(ComposerAction::None)
            }
            KeyCode::Enter if !key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.execute_selected_command()
            }
            _ => None,
        }
    }
}

fn accepts_character(character: char, modifiers: KeyModifiers) -> bool {
    let alt_gr = modifiers.contains(KeyModifiers::CONTROL | KeyModifiers::ALT);
    !character.is_control()
        && !matches!(character, '\u{2028}' | '\u{2029}')
        && (!modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) || alt_gr)
}
