use super::ComposerState;
use crate::frontend::composer::action::ComposerAction;
use crate::frontend::composer::slash_commands::{
    SlashCommand, SlashPaletteWidget, matching_commands, palette_row_count, slash_args, slash_query,
};

impl ComposerState {
    pub fn palette_visible(&self) -> bool {
        !self.palette_dismissed && slash_query(&self.text).is_some()
    }

    pub fn palette_height(&self, max_height: u16) -> u16 {
        if !self.palette_visible() {
            return 0;
        }
        palette_row_count(self.palette_matches().len(), max_height)
    }

    pub fn palette_widget(&self) -> Option<SlashPaletteWidget> {
        self.palette_visible()
            .then(|| SlashPaletteWidget::new(self.palette_matches(), self.palette_selected))
    }

    pub(super) fn palette_matches(&self) -> Vec<&'static SlashCommand> {
        slash_query(&self.text)
            .map(matching_commands)
            .unwrap_or_default()
    }

    fn selected_command(&self) -> Option<&'static SlashCommand> {
        self.palette_matches().get(self.palette_selected).copied()
    }

    pub(super) fn move_palette(&mut self, delta: isize) {
        let count = self.palette_matches().len();
        if count == 0 {
            return;
        }
        let next = self.palette_selected as isize + delta;
        self.palette_selected = next.rem_euclid(count as isize) as usize;
    }

    pub(super) fn complete_selected_command(&mut self) {
        let Some(command) = self.selected_command() else {
            return;
        };
        let args = slash_args(&self.text);
        let mut text = format!("/{}", command.name);
        if !args.is_empty() {
            text.push(' ');
            text.push_str(&args);
        } else if command.argument.is_some() {
            text.push(' ');
        }
        self.text = text;
        self.cursor = self.text.len();
        self.vertical_column = None;
        self.palette_selected = 0;
    }

    pub(super) fn execute_selected_command(&mut self) -> Option<ComposerAction> {
        let command = self.selected_command()?;
        let args = slash_args(&self.text);
        let name = command.name.to_string();
        *self = Self::default();
        Some(ComposerAction::Command { name, args })
    }

    pub(super) fn on_text_changed(&mut self) {
        if slash_query(&self.text).is_none() {
            self.palette_dismissed = false;
        }
        self.palette_selected = 0;
    }
}
