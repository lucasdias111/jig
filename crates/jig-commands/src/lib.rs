//! The command layer: presets, the floating command input, and (later) the
//! reply bubble and diff preview.

pub mod bubble;
pub mod motion;
pub mod new_command;
pub mod palette;
pub mod presets;

pub use bubble::Bubble;
pub use new_command::{NewCommandEvent, NewCommandForm};
pub use palette::{CommandPalette, PaletteEvent};
pub use presets::{CommentMode, Invocation, Preset, Scope};
