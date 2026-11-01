//! The user's servers file, and finding a server's markers and program on
//! disk.

use std::path::{Path, PathBuf};

use super::session::server_named;
use crate::domain::servers::Server;

/// The user's `language-servers.toml`; `None` in tests.
pub fn user_path() -> Option<PathBuf> {
    jig_tools::user_file("language-servers.toml")
}

impl Server {
    /// Whether `dir` holds one of the markers.
    pub fn marks(&self, dir: &Path) -> bool {
        self.root_markers
            .iter()
            .any(|marker| match marker.strip_prefix("*.") {
                Some(extension) => std::fs::read_dir(dir).is_ok_and(|entries| {
                    entries.flatten().any(|entry| {
                        Path::new(&entry.file_name())
                            .extension()
                            .is_some_and(|ext| ext == extension)
                    })
                }),
                None => dir.join(marker).exists(),
            })
    }
}

/// `server`'s own folder in the servers folder, whether or not it's there.
pub fn server_dir(server: &Server) -> Option<PathBuf> {
    jig_tools::servers_dir().map(|dir| dir.join(&server.name))
}

/// The first of `server`'s commands that's installed, as its program and
/// arguments.
pub fn program_for(server: &Server) -> Option<(PathBuf, &[String])> {
    let path = jig_tools::search_path();
    let own = server_dir(server);
    server.commands.iter().find_map(|argv| {
        let (name, args) = argv.split_first()?;
        let program = jig_tools::find_program(name, path)
            .or_else(|| {
                jig_tools::bin_dirs(own.as_ref()?)
                    .iter()
                    .find_map(|dir| jig_tools::executable_in(dir, name))
            })
            // Debuggers' installers may have put it with them.
            .or_else(|| {
                jig_tools::debuggers_dir()
                    .map(|dir| dir.join("bin").join(name))
                    .filter(|program| program.is_file())
            })?;
        Some((program, args))
    })
}

/// Whether `server`, by name, is installed.
pub fn is_installed(server: &str) -> bool {
    server_named(server).is_some_and(|s| program_for(s).is_some())
}
