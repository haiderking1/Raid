mod action;
mod dock;
mod input;
mod metrics;
mod state;
mod widget;
mod wrap;

pub mod slash_commands;

pub use action::ComposerAction;
pub use dock::docked_layout;
pub use input::paint_input_editor;
pub use metrics::padded_input_layout;
pub use state::ComposerState;
pub use widget::ComposerWidget;
pub use wrap::visual_lines_for_cursor;
