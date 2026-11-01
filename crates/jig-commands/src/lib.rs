//! The AI layer's UI: the floating input that starts a conversation, the
//! bubble for notes and errors, and the agent conversation, with the
//! panel styling and pickers the rest of Jig shares.
//!
//! This file is the crate's facade: it only says what the rest of Jig may
//! use. The work is in its layers (`docs/layers.md`):
//!
//! - [`ui`]: the input, the conversation, the bubble, panel styling and
//!   motion, as GPUI views.
//! - [`domain`]: the conversation's transcript, the input's lists and
//!   fuzzy matching, in plain Rust.
//! - [`store`]: the project's files on disk.

mod domain;
mod store;
mod ui;

// The crate's contract: what the rest of Jig uses of it, by the same paths
// as before it was layered.
pub use domain::fuzzy;
pub use domain::palette::{AgentCommandInfo, ModelChoice, ModelMenu, PastConversation};
pub use domain::transcript::{
    ChatEntry, ChatModels, ChatStatus, DiffLine, DrawingAction, LiveStep,
};
pub use ui::bubble::Bubble;
pub use ui::chat::{ChatDrag, ChatFrame, ChatResize, Conversation, LinkHandler};
pub use ui::palette::{CommandPalette, PaletteEvent};
pub use ui::{bubble, chat, motion, palette, surface, windows};

/// The project's files as the pickers list them.
pub mod project_files {
    pub use crate::domain::paths::relative_path;
    pub use crate::store::persist::walk;
}
