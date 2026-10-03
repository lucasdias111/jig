//! The command layer: presets, the floating command input, and (later) the
//! reply bubble and diff preview.

gpui_kit::actions!(
    jig_commands,
    [
        /// Cmd+E: let a command explore the project (the form's setting, or
        /// one run from the command input).
        ToggleExplore
    ]
);

pub mod bubble;
pub mod motion;
pub mod new_command;
pub mod palette;
pub mod presets;

pub use bubble::{Bubble, LiveStep};
pub use new_command::{NewCommandEvent, NewCommandForm};
pub use palette::{CommandPalette, PaletteEvent};
pub use presets::{CommentMode, Invocation, Preset, Scope};
