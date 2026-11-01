//! The formatters as data: `assets/default-formatters.toml` and the user's
//! `formatters.toml`, merged, and looked up by language or key.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, bail};
use serde::Deserialize;

use jig_tools::Help;

const BUILT_IN: &str = include_str!("../../../../assets/default-formatters.toml");

/// A starter for the user's `formatters.toml`.
pub const USER_TEMPLATE: &str = r#"# Your formatters, used when a file's language server doesn't format. One
# with the same key as a built-in replaces it; see
# assets/default-formatters.toml in Jig's source for the built-ins. A
# formatter reads the text on stdin and writes it formatted to stdout;
# ${file} is the file's path and ${root} the project's folder.
#
# [[formatter]]
# key = "clang-format"
# name = "clang-format"
# languages = ["c", "cpp"]
# command = ["clang-format", "--assume-filename", "${file}"]
"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FormatterFile {
    #[serde(default)]
    formatter: Vec<Formatter>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Formatter {
    pub key: String,
    pub name: String,
    /// Jig's names for the languages it formats.
    pub languages: Vec<String>,
    pub(crate) command: Vec<String>,
    /// A shell command installing it into `${dir}`, its own folder in
    /// Jig's formatters folder.
    pub(crate) install: Option<String>,
    pub(crate) help: Option<Help>,
}

impl Formatter {
    pub fn installable(&self) -> bool {
        self.install.is_some()
    }

    /// How to get it, for when it isn't installed.
    pub fn help(&self) -> String {
        self.help.as_ref().map_or_else(
            || format!("Install {} and put it on the PATH.", self.name),
            Help::text,
        )
    }

    /// The arguments, with `${file}` and `${root}` filled in.
    pub(crate) fn args(&self, file: &Path, root: &Path) -> Vec<String> {
        self.command[1..]
            .iter()
            .map(|arg| {
                arg.replace("${file}", &file.to_string_lossy())
                    .replace("${root}", &root.to_string_lossy())
            })
            .collect()
    }
}

/// The formatters, built-in and the user's, and what was wrong with the
/// user's file if anything.
pub struct Registry {
    pub formatters: Vec<Arc<Formatter>>,
    pub error: Option<String>,
}

impl Registry {
    /// Every formatter for `language`, installed or not, first first.
    pub fn serving(&self, language: &str) -> impl Iterator<Item = &Arc<Formatter>> {
        self.formatters
            .iter()
            .filter(move |f| f.languages.iter().any(|l| l == language))
    }

    /// A formatter by key.
    pub fn get(&self, key: &str) -> Option<Arc<Formatter>> {
        self.formatters.iter().find(|f| f.key == key).cloned()
    }

    /// Whether any formatter, installed or not, is for `language`.
    pub fn knows(&self, language: &str) -> Option<Arc<Formatter>> {
        self.formatters
            .iter()
            .find(|f| f.languages.iter().any(|l| l == language))
            .cloned()
    }
}

/// The formatters in `text`, a `formatters.toml`.
pub fn parse(text: &str) -> Result<Vec<Formatter>> {
    let file: FormatterFile = toml::from_str(text)?;
    for formatter in &file.formatter {
        if formatter.command.is_empty() {
            bail!("formatter “{}” needs a `command`", formatter.key);
        }
    }
    Ok(file.formatter)
}

/// The built-in formatters with `user`, the text of the user's file,
/// merged in.
pub fn merge(user: Option<&str>) -> Registry {
    let mut formatters = parse(BUILT_IN).expect("the built-in formatters parse");
    let mut error = None;
    if let Some(text) = user {
        match parse(text) {
            Ok(own) => {
                // The user's come first, so one of theirs for a language
                // wins over a built-in for it.
                let mut merged = Vec::new();
                for formatter in own {
                    formatters.retain(|f| f.key != formatter.key);
                    merged.push(formatter);
                }
                merged.append(&mut formatters);
                formatters = merged;
            }
            Err(e) => error = Some(format!("formatters.toml isn't valid: {e:#}")),
        }
    }
    Registry {
        formatters: formatters.into_iter().map(Arc::new).collect(),
        error,
    }
}
