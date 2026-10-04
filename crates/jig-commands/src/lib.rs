//! The command layer: presets, the floating command input, the reply bubble
//! and the agent conversation.

pub mod bubble;
pub mod chat;
pub mod motion;
pub mod new_command;
pub mod palette;
pub mod presets;
pub mod surface;

pub use bubble::{Bubble, LiveStep};
pub use chat::{ChatBounds, ChatDrag, ChatEntry, ChatStatus, Conversation};
pub use new_command::{NewCommandEvent, NewCommandForm};
pub use palette::{CommandPalette, PaletteEvent};
pub use presets::{CommentMode, Invocation, Preset, Scope};
