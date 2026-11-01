//! Editing help in a buffer: documents, edits made in one undo step, snippets,
//! completions, hover and find in the file.
//!
//! This file is the crate's facade: it only says what the rest of Jig may
//! use, by the same paths as before it was layered. The work is in its
//! layers (`docs/layers.md`):
//!
//! - [`ui`]: the find panel, as a GPUI view.
//! - [`domain`]: completions, snippets, hover and signature help, and
//!   text edits, in plain Rust.
//! - [`store`]: the file on disk, and the snippets as read.

mod domain;
mod store;
mod ui;

pub use domain::{completions, hover, text_edits};
pub use ui::find_panel;

/// The file behind the editor.
pub mod document {
    pub use crate::store::persist::{Document, atomic_write};
}

/// Snippets: parsing and expanding them, and Jig's for each language.
pub mod snippets {
    pub use crate::domain::snippets::*;
    pub use crate::store::cache::{for_language, user_path};
}
