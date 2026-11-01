//! A language server, as `assets/default-languages.toml` describes it.

use serde::Deserialize;

/// A starter for the user's `language-servers.toml`.
pub const USER_TEMPLATE: &str = r#"# Your language servers. One with the same name as a built-in replaces it;
# others are added, and a server here for a language wins over the built-in
# one. Restart Jig after changing this file. The fields are the ones in
# assets/default-languages.toml in Jig's source, which lists the built-ins.
#
# [[server]]
# name = "pylsp"
# languages = ["python"]
# commands = [["pylsp"]]
# root_markers = ["pyproject.toml", "setup.py"]
#
# A server with no commands turns a language's server off:
#
# [[server]]
# name = "none"
# languages = ["markdown"]
# commands = []
"#;

/// A language server, as `assets/default-languages.toml` describes it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub name: String,
    /// Jig's names for the languages it serves.
    pub languages: Vec<String>,
    /// Commands to try, in order; the first installed one is run.
    pub commands: Vec<Vec<String>>,
    /// Files whose folder is the project root; `*.ext` matches any file
    /// with that extension.
    #[serde(default)]
    pub root_markers: Vec<String>,
    /// Take the outermost folder with a marker rather than the nearest: a
    /// Cargo or Go workspace contains its members' manifests.
    #[serde(default)]
    pub outermost: bool,
    /// A shell command installing it into `${dir}`, its own folder.
    pub install: Option<String>,
    help: Option<jig_tools::Help>,
}

impl Server {
    pub fn installable(&self) -> bool {
        self.install.is_some()
    }

    /// How to get it, for when it isn't installed.
    pub fn help(&self) -> String {
        self.help.as_ref().map_or_else(
            || format!("Install {} and put it on the PATH.", self.name),
            jig_tools::Help::text,
        )
    }
}
