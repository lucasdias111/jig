//! Run configurations: the commands a project is run, tested and built
//! with, as IntelliJ has them.
//!
//! They come from the project's `.jig/run.toml`, and from what the project
//! itself declares: Cargo binaries and examples, `package.json` scripts, a
//! Go module. A configuration in the file with the same name as a detected
//! one replaces it.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::Deserialize;

/// The project's own configurations, relative to its root.
pub const FILE: &str = ".jig/run.toml";

/// A starter for a new `.jig/run.toml`.
pub const TEMPLATE: &str = r#"# Run configurations for this project. Pick one with ⌃⌥R, run it with ⌃R,
# stop it with ⌘F2. Jig also lists the Cargo binaries, package.json scripts
# and Go module it finds; one here with the same name replaces it.
#
# command  runs through /bin/sh (cmd on Windows), from the project root unless `cwd` says
#          otherwise (relative to the root).
# env      is added to the environment.
#
# Debugging (⌃D) works on its own for commands a debugger takes, such as
# `cargo run`, `npm run dev`, `python3 app.py` or `go run .`. Otherwise:
# program   what to debug, relative to the root: an executable or a script;
# args      its arguments;
# build     a shell command that builds it first;
# debugger  which debugger to use, by its key in Settings.

[[run]]
name = "Run"
command = ""
# cwd = "."
# env = { RUST_LOG = "debug" }
# program = "target/debug/app"
# args = ["--verbose"]
# build = "make"
# debugger = "rust"
"#;

#[derive(Clone, Debug, PartialEq)]
pub struct RunConfig {
    pub name: String,
    /// A shell command line.
    pub command: String,
    /// Absolute.
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub source: Source,
    /// What to debug, when the file says.
    pub debug: Option<DebugProgram>,
    /// The debugger to use, by key, when the file says.
    pub debugger: Option<String>,
}

/// An executable to debug, from `program`, `args` and `build` in the file.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugProgram {
    /// Absolute.
    pub program: PathBuf,
    pub args: Vec<String>,
    /// A shell command to build it with first.
    pub build: Option<String>,
}

/// Where a configuration came from, shown beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    File,
    Cargo,
    Npm,
    Go,
    Maven,
    Gradle,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::File => "run.toml",
            Source::Cargo => "Cargo",
            Source::Npm => "package.json",
            Source::Go => "Go",
            Source::Maven => "Maven",
            Source::Gradle => "Gradle",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunFile {
    #[serde(default)]
    run: Vec<FileEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileEntry {
    name: String,
    command: String,
    cwd: Option<PathBuf>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    program: Option<PathBuf>,
    #[serde(default)]
    args: Vec<String>,
    build: Option<String>,
    debugger: Option<String>,
}

/// The configurations in `text`, a `.jig/run.toml` in `root`.
pub fn parse_file(root: &Path, text: &str) -> Result<Vec<RunConfig>> {
    let file: RunFile = toml::from_str(text).with_context(|| format!("{FILE} is not valid"))?;
    Ok(file
        .run
        .into_iter()
        .filter(|entry| !entry.command.trim().is_empty())
        .map(|entry| RunConfig {
            name: entry.name,
            command: entry.command,
            cwd: entry
                .cwd
                .map_or_else(|| root.to_path_buf(), |cwd| root.join(cwd)),
            env: entry.env.into_iter().collect(),
            source: Source::File,
            debug: entry.program.map(|program| DebugProgram {
                program: root.join(program),
                args: entry.args,
                build: entry.build.filter(|build| !build.trim().is_empty()),
            }),
            debugger: entry.debugger,
        })
        .collect())
}

/// A detected configuration, run in `root`.
pub(crate) fn config(root: &Path, name: String, command: String, source: Source) -> RunConfig {
    RunConfig {
        name,
        command,
        cwd: root.to_path_buf(),
        env: Vec::new(),
        source,
        debug: None,
        debugger: None,
    }
}

#[derive(Deserialize)]
pub(crate) struct Metadata {
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    targets: Vec<Target>,
}

#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
}

pub(crate) fn cargo_configs(root: &Path, metadata: &Metadata) -> Vec<RunConfig> {
    let mut configs = Vec::new();
    for kind in ["bin", "example"] {
        for package in &metadata.packages {
            for target in package
                .targets
                .iter()
                .filter(|target| target.kind.iter().any(|k| k == kind))
            {
                let (name, flag) = match kind {
                    "bin" => (target.name.clone(), "--bin"),
                    _ => (format!("{} (example)", target.name), "--example"),
                };
                let command = format!(
                    "cargo run -p {} {flag} {}",
                    shell_word(&package.name),
                    shell_word(&target.name)
                );
                configs.push(config(root, name, command, Source::Cargo));
            }
        }
    }
    configs.push(config(
        root,
        "Test".into(),
        "cargo test".into(),
        Source::Cargo,
    ));
    configs.push(config(
        root,
        "Build".into(),
        "cargo build".into(),
        Source::Cargo,
    ));
    configs
}

/// Detected names can clash, e.g. a `test` script and Cargo's tests; the
/// later ones get their source added.
pub(crate) fn dedupe_names(configs: &mut [RunConfig]) {
    let mut seen = HashSet::new();
    for config in configs {
        if !seen.insert(config.name.to_lowercase()) {
            config.name = format!("{} ({})", config.name, config.source.label());
            seen.insert(config.name.to_lowercase());
        }
    }
}

/// `command` split into words as `sh` would, or `None` when it does more
/// than run one program: pipes, `&&`, variables, redirections.
pub fn split_command(command: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => word.push(chars.next()?),
                        '$' | '`' => return None,
                        c => word.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.push(chars.next()?);
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '|' | '&' | ';' | '<' | '>' | '$' | '`' | '(' | ')' | '*' | '?' => return None,
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    (!words.is_empty()).then_some(words)
}

/// `word` quoted for `sh` if it needs it.
pub fn shell_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.:/@+=,".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}
