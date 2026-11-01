//! The file tree and its icons.
//!
//! This file is the crate's facade: it only says what the rest of Jig may
//! use. The work is in its layers (`docs/layers.md`):
//!
//! - [`ui`]: the sidebar's tree and the file icons.
//! - [`domain`]: the rules on names and moved paths, in plain Rust.
//! - [`store`]: the folders' listings, and changing files on disk.

mod domain;
mod store;
mod ui;

pub use ui::file_icons;

/// The project sidebar: every folder and file under the project root.
pub mod file_tree {
    pub use crate::domain::paths::{NewEntry, moved_path};
    pub use crate::ui::file_tree::{FileTree, FileTreeEvent, key_bindings};
}
