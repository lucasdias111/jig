//! Go to File and Find in Files.
//!
//! This file is the crate's facade: it only says what the rest of Jig may
//! use, by the same paths as before it was layered. The work is in its
//! layers (`docs/layers.md`):
//!
//! - [`ui`]: Go to File and Find in Files, as GPUI views.
//! - [`domain`]: Go to File's list and searching text, in plain Rust.
//! - [`store`]: the project's files on disk.

mod domain;
mod store;
mod ui;

pub use ui::{find_in_files, quick_open};

/// Searching the text of every file in a project.
pub mod project_search {
    pub use crate::domain::text_search::{
        LineMatch, MAX_RESULTS, SearchOptions, SearchResults, line_match, pattern,
    };
}
