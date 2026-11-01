//! The languages Jig can highlight, and which file each one applies to:
//! `assets/default-languages.toml`, which also lists the language servers.
//!
//! Grammars are compiled in (the `gpui-kit` features in the workspace
//! `Cargo.toml`, and the crates in `ui/grammars.rs`); Settings adds
//! languages to the list from its Library, turns them on or off and changes
//! their extensions.
//!
//! This file is the crate's facade. The work is in its layers:
//!
//! - [`ui`]: registering the grammars with the highlighter.
//! - [`domain`]: the table of languages and servers, and matching files.
//! - [`store`]: the table as read at startup, and servers' programs on disk.

mod domain;
mod store;
mod ui;

use std::path::Path;

use jig_settings::LanguageSettings;

pub use domain::languages::{Language, format_extensions, parse_extensions};
pub use domain::servers::{Server, USER_TEMPLATE};
pub use store::persist::{is_installed, program_for, server_dir, user_path};
pub use store::session::{bundled, get, line_comment, server_named, servers, user_error};
pub use ui::register_grammars;

/// The language to highlight `path` with, or `"text"` for none.
pub fn language_for(path: &Path, settings: &LanguageSettings) -> &'static str {
    domain::languages::language_for(bundled(), path, settings)
}

/// The language `path` is written in, whether or not it's highlighted.
/// `None` for a language that isn't added.
pub fn written_in(path: &Path, settings: &LanguageSettings) -> Option<&'static Language> {
    domain::languages::written_in(bundled(), path, settings)
}

#[cfg(test)]
mod tests;
