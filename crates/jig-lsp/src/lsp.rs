//! Language servers, as Neovim runs them: a table saying which server
//! serves which languages and which files mark a project's root, one server
//! per root shared by every window, and documents kept in sync as they're
//! edited.
//!
//! Servers aren't bundled. One that isn't installed is simply not used, and
//! Cmd+click falls back to [`crate::definitions`].

pub use crate::domain::positions::{Encoding, offset, position, range};
pub use crate::domain::protocol::{Formatting, locations, text_document_position};
pub use crate::domain::uri::{decode, encode, file_uri, uri_path};
pub use crate::store::client::{Client, Response};
pub use crate::store::servers::{language_id, root_for, server_for};
#[cfg(any(test, feature = "testing"))]
pub use crate::ui::registry::register;
pub use crate::ui::registry::{
    Failures, Registry, ServersChanged, client_for, failure_for, is_current, servers_changed,
};

#[cfg(any(test, feature = "testing"))]
pub use crate::store::client::testing::fake_server;

/// The problems language servers report, kept per file.
pub mod diagnostics {
    pub use crate::domain::diagnostics::Published;
    pub use crate::store::diagnostics::{parse, path_key, uri_key};
    pub use crate::ui::diagnostics::{Diagnostics, Stored, entries, publish};
}

/// Installing language servers from Settings.
pub mod install {
    pub use crate::store::install::{install, installed_by_jig, status, uninstall};
}

#[cfg(test)]
mod tests;
