//! The command layer: presets, the floating command input, and (later) the
//! reply bubble and diff preview.

pub mod bubble;
pub mod palette;
pub mod presets;

pub use bubble::Bubble;
pub use palette::{CommandPalette, PaletteEvent};
pub use presets::{Invocation, Preset, Scope};
