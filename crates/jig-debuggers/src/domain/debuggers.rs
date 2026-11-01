//! The debuggers as data: `assets/default-debuggers.toml` and the user's
//! `debuggers.toml`, checked and merged.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::Value;

const BUILT_IN: &str = include_str!("../../../../assets/default-debuggers.toml");

/// A starter for the user's `debuggers.toml`.
pub const USER_TEMPLATE: &str = r#"# Your debuggers. One with the same key as a built-in replaces it; see
# assets/default-debuggers.toml in Jig's source for the built-ins and every
# field. Turn new ones on in Settings, under Languages.
#
# [[debugger]]
# key = "ruby"
# name = "Ruby"
# languages = ["ruby"]
# server = ["rdbg", "--open", "--port", "${port}", "--", "ruby", "${program}"]
# commands = ["ruby"]
# extensions = ["rb"]
# help = "Install it with gem install debug."
#
# [debugger.launch]
# type = "rdbg"
# request = "attach"
# name = "${name}"
# cwd = "${cwd}"
"#;

/// Recent toolchains sum up every struct on one line from all its fields,
/// through every reference, with no limit: for a `&mut` to a big app's
/// state that never ends, and LLDB answers nothing else meanwhile, so the
/// program can't be resumed. Structs still open field by field. The
/// result is kept, so the console doesn't show it.
pub const WITHOUT_STRUCT_SUMMARIES: &str = "script _ = lldb.debugger.GetCategory(\"Rust\").DeleteTypeSummary(lldb.SBTypeNameSpecifier(\"lldb_lookup.is_udt\", lldb.eFormatterMatchCallback))";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DebuggerFile {
    #[serde(default)]
    debugger: Vec<Debugger>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Debugger {
    /// Its key in `settings.toml`.
    pub key: String,
    pub name: String,
    /// The languages, by Jig's names, whose files take breakpoints.
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub(crate) adapter: Vec<String>,
    #[serde(default)]
    pub(crate) server: Vec<String>,
    /// Asks this language server, as Jig runs it, for the adapter's port.
    pub(crate) lsp: Option<String>,
    /// The `workspace/executeCommand` that answers with the port.
    lsp_start: Option<String>,
    /// Plugins the language server loads at start; `*` matches in a file
    /// name.
    #[serde(default)]
    pub(crate) lsp_bundles: Vec<String>,
    #[serde(default)]
    pub(crate) adapter_env: BTreeMap<String, String>,
    adapter_id: Option<String>,
    pub(crate) check: Option<Vec<String>>,
    pub(crate) install: Option<String>,
    /// What `install` puts in the debuggers folder, relative to it, for
    /// Uninstall to move to the Trash.
    #[serde(default)]
    pub(crate) uninstall: Vec<String>,
    help: Option<jig_tools::Help>,
    /// How the commands it debugs start, as words.
    #[serde(default)]
    commands: Vec<String>,
    /// Extensions of a run.toml `program` it debugs.
    #[serde(default)]
    extensions: Vec<String>,
    /// Takes any other `program`, as a native executable.
    #[serde(default)]
    executables: bool,
    /// `"cargo"`: built with `cargo build` before debugging.
    build: Option<String>,
    launch: toml::Table,
    launch_program: Option<toml::Table>,
}

impl Debugger {
    pub fn adapter_id(&self) -> &str {
        self.adapter_id.as_deref().unwrap_or(&self.key)
    }

    pub fn installable(&self) -> bool {
        self.install.is_some()
    }

    pub fn builds_with_cargo(&self) -> bool {
        self.build.as_deref() == Some("cargo")
    }

    pub fn commands(&self) -> &[String] {
        &self.commands
    }

    pub fn takes_extension(&self, extension: &str) -> bool {
        self.extensions.iter().any(|e| e == extension)
    }

    pub fn takes_executables(&self) -> bool {
        self.executables
    }

    /// How to get it, for this system.
    pub fn help(&self) -> String {
        match &self.help {
            None => format!("Jig couldn't find {}'s debugger.", self.name),
            Some(help) => help.text(),
        }
    }

    /// The `launch` template: the `program` one when the run configuration
    /// gave a program and there is one.
    pub fn launch_template(&self, from_program: bool) -> Value {
        let table = match (&self.launch_program, from_program) {
            (Some(program), true) => program,
            _ => &self.launch,
        };
        serde_json::to_value(table).unwrap_or(Value::Null)
    }

    /// The language server it asks for its adapter, and the command that
    /// does it, when it's started that way.
    pub fn lsp(&self) -> Option<(&str, &str)> {
        Some((self.lsp.as_deref()?, self.lsp_start.as_deref()?))
    }

    /// The adapter's words, and whether it serves on a port, rather than
    /// on its stdin and stdout.
    pub(crate) fn adapter_words(&self) -> (&[String], bool) {
        if self.server.is_empty() {
            (&self.adapter, false)
        } else {
            (&self.server, true)
        }
    }

    fn validate(&self) -> Result<()> {
        let ways = [
            !self.adapter.is_empty(),
            !self.server.is_empty(),
            self.lsp.is_some(),
        ];
        if ways.into_iter().filter(|way| *way).count() != 1 {
            bail!(
                "debugger “{}” needs one of `adapter`, `server` or `lsp`",
                self.key
            );
        }
        if self.lsp.is_some() != self.lsp_start.is_some() {
            bail!("debugger “{}”: `lsp` goes with `lsp_start`", self.key);
        }
        if !self.server.is_empty() && !self.server.iter().any(|arg| arg.contains("${port}")) {
            bail!("debugger “{}”: `server` must pass ${{port}}", self.key);
        }
        if let Some(build) = &self.build
            && build != "cargo"
        {
            bail!("debugger “{}”: the only `build` is \"cargo\"", self.key);
        }
        Ok(())
    }
}

/// A ready-to-run adapter.
#[derive(Clone, Debug, PartialEq)]
pub struct AdapterCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Serves DAP on a TCP port, rather than on its stdin and stdout.
    pub server: bool,
    /// What Settings says it's using.
    pub shown: PathBuf,
}

/// The debuggers, built-in and the user's, and what was wrong with the
/// user's file if anything.
pub struct Registry {
    pub debuggers: Vec<Arc<Debugger>>,
    pub error: Option<String>,
}

impl Registry {
    pub fn get(&self, key: &str) -> Option<Arc<Debugger>> {
        self.debuggers.iter().find(|d| d.key == key).cloned()
    }

    /// The debugger for files in `language`, if there is one.
    pub fn for_language(&self, language: &str) -> Option<Arc<Debugger>> {
        self.debuggers
            .iter()
            .find(|d| d.languages.iter().any(|l| l == language))
            .cloned()
    }
}

/// The debuggers in `text`, a `debuggers.toml`, each checked.
pub fn parse(text: &str) -> Result<Vec<Debugger>> {
    let file: DebuggerFile = toml::from_str(text)?;
    for debugger in &file.debugger {
        debugger.validate()?;
    }
    Ok(file.debugger)
}

/// The built-in debuggers with `user`, the text of the user's file,
/// merged in: theirs replace a built-in by key.
pub fn merge(user: Option<&str>) -> Registry {
    let mut debuggers = parse(BUILT_IN).expect("the built-in debuggers parse");
    let mut error = None;
    if let Some(text) = user {
        match parse(text) {
            Ok(own) => {
                for debugger in own {
                    match debuggers.iter_mut().find(|d| d.key == debugger.key) {
                        Some(built_in) => *built_in = debugger,
                        None => debuggers.push(debugger),
                    }
                }
            }
            Err(e) => error = Some(format!("debuggers.toml isn't valid: {e:#}")),
        }
    }
    Registry {
        debuggers: debuggers.into_iter().map(Arc::new).collect(),
        error,
    }
}
