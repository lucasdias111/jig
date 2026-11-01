//! The editor adapter.
//!
//! The command layer talks to the text editor only through [`EditorHandle`].
//! Today it is backed by GPUI Kit's code editor; swapping in Jig's own editor
//! element later means writing another implementation of this trait.

mod ui;

pub use ui::{EditorHandle, KitEditor, selection_bounds};
