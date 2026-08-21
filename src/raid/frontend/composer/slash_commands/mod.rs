mod palette;
mod query;
mod registry;

pub use palette::SlashPaletteWidget;
pub use query::{matching_commands, palette_row_count, slash_args, slash_query};
pub use registry::SlashCommand;

#[cfg(test)]
pub use registry::COMMANDS;
