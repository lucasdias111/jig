//! The AI layer's UI: the floating input that starts a conversation, the
//! bubble for notes and errors, and the agent conversation.

pub mod bubble;
pub mod chat;
pub mod fuzzy;
pub mod motion;
pub mod palette;
pub mod project_files;
pub mod surface;
pub mod windows;

pub use bubble::{Bubble, LiveStep};
pub use chat::{
    ChatDrag, ChatEntry, ChatFrame, ChatModels, ChatResize, ChatStatus, Conversation, DiffLine,
    DrawingAction, LinkHandler,
};
pub use palette::{
    AgentCommandInfo, CommandPalette, ModelChoice, ModelMenu, PaletteEvent, PastConversation,
};
