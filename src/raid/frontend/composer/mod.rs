mod action;
mod dock;
mod metrics;
mod state;
mod widget;
mod wrap;

pub mod slash_commands;

pub use action::ComposerAction;
pub use dock::docked_layout;
pub use metrics::{composer_input_layout, padded_input_layout, InputRowLayout};
pub use state::ComposerState;
pub use widget::ComposerWidget;
pub use wrap::{visual_lines_for_cursor, ComposerLayout};
