//! Where Jig's AI meets the world: the agents' processes and servers,
//! spoken to over stdio and HTTP, the files and keychain it keeps, and
//! what lasts as long as this process.
//!
//! The protocol clients sit here rather than in a layer of their own: each
//! is the live state of a conversation with an agent, kept as long as it
//! runs, and what they say is read and answered by `domain`.

pub mod acp;
pub mod harness;
mod http;
mod models;
pub mod opencode;
pub mod persist;
pub mod session;

use std::path::Path;

/// `directory` as the agents know it: the real path, as the agent's own
/// tools report it.
pub(crate) fn canonical(directory: &Path) -> String {
    directory
        .canonicalize()
        .unwrap_or_else(|_| directory.to_path_buf())
        .to_string_lossy()
        .into_owned()
}
