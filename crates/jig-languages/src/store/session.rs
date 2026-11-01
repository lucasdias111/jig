//! The languages and servers, read once at startup with the user's
//! servers and kept for the life of the app.

use std::sync::LazyLock;

use super::persist::user_path;
use crate::domain::languages::{Language, Table, load};
use crate::domain::servers::Server;

static TABLE: LazyLock<Table> = LazyLock::new(|| {
    let user = user_path().and_then(|path| std::fs::read_to_string(path).ok());
    load(user.as_deref())
});

/// Every bundled language: the starters, then the ones to add from
/// Settings. Where two claim an extension, the first wins.
pub fn bundled() -> &'static [Language] {
    &TABLE.languages
}

/// Every language server Jig knows how to run, the user's first.
pub fn servers() -> &'static [Server] {
    &TABLE.servers
}

/// What was wrong with the user's `language-servers.toml` at startup.
pub fn user_error() -> Option<&'static str> {
    TABLE.user_error.as_deref()
}

/// A bundled language by name.
pub fn get(name: &str) -> Option<&'static Language> {
    bundled().iter().find(|language| language.name == name)
}

/// What starts a line comment in `language`, Jig's name for it. `None` for
/// languages without one, such as JSON.
pub fn line_comment(language: &str) -> Option<&'static str> {
    get(language)?.line_comment
}

/// A server by name.
pub fn server_named(name: &str) -> Option<&'static Server> {
    servers().iter().find(|s| s.name == name)
}
