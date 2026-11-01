//! Finding the programs and tools debuggers need on this computer: an
//! adapter's program, `lldb-dap`, js-debug, Rust's pretty-printers.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use jig_process::NoWindow as _;
use jig_tools::{data_dir, debuggers_dir};
use serde_json::{Value, json};

use crate::domain::debuggers::{AdapterCommand, Debugger, WITHOUT_STRUCT_SUMMARIES};
use crate::domain::vars::{Tools, Vars};

/// The tools installed on this computer, as `${variables}`.
struct Installed;

impl Tools for Installed {
    fn find(&self, name: &str, cwd: &Path) -> Option<Result<Value, String>> {
        let path = |path: Option<PathBuf>, missing: &str| {
            path.map(|path| json!(path))
                .ok_or_else(|| missing.to_string())
        };
        Some(match name {
            "debuggers" => path(debuggers_dir(), "There's no home folder for debuggers."),
            "node" => path(program_on_path("node"), "Node isn't installed."),
            "lldb_dap" => path(lldb_dap(), "lldb-dap isn't installed."),
            "js_debug" => path(js_debug(), "js-debug isn't installed."),
            "lldb_rust_formatters" => Ok(json!(rust_init_commands(cwd))),
            _ => return None,
        })
    }
}

impl Vars {
    /// Variables for a debugger run in `cwd`, with this computer's tools.
    pub fn new(cwd: &Path) -> Self {
        Vars::with_tools(cwd, &Installed)
    }
}

impl Debugger {
    /// The adapter to start, with variables filled in, or what's missing.
    pub fn adapter_command(&self, vars: &Vars) -> Result<AdapterCommand, String> {
        let (words, server) = self.adapter_words();
        let words = words
            .iter()
            .map(|word| vars.expand_str(word))
            .collect::<Result<Vec<_>, _>>()?;
        let program = resolve_program(&words[0])?;
        let env = self
            .adapter_env
            .iter()
            .map(|(key, value)| Ok((key.clone(), vars.expand_str(value)?)))
            .collect::<Result<Vec<_>, String>>()?;
        // Shown in Settings: js-debug's folder says more than Node's path.
        let shown = if words.iter().any(|w| w.contains("dapDebugServer.js")) {
            vars.get("js_debug")
                .ok()
                .and_then(|v| v.as_str().map(PathBuf::from))
                .unwrap_or_else(|| program.clone())
        } else {
            program.clone()
        };
        Ok(AdapterCommand {
            program,
            args: words[1..].to_vec(),
            env,
            server,
            shown,
        })
    }
}

/// A program by name: on the `PATH`, or in `${debuggers}/bin`. A path must
/// exist.
pub(crate) fn resolve_program(word: &str) -> Result<PathBuf, String> {
    if word.contains('/') {
        let path = PathBuf::from(word);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(format!("Nothing at {word}."))
        };
    }
    program_on_path(word)
        .or_else(|| {
            debuggers_dir()
                .map(|dir| dir.join("bin").join(word))
                .filter(|path| path.is_file())
        })
        .ok_or_else(|| format!("{word} isn't installed."))
}

/// `lldb-dap` on the `PATH`, under any of the names distributions give it,
/// or on macOS the one Xcode has.
pub fn lldb_dap() -> Option<PathBuf> {
    static ADAPTER: OnceLock<Option<PathBuf>> = OnceLock::new();
    ADAPTER
        .get_or_init(|| {
            if let Some(path) = ["lldb-dap", "lldb-vscode"]
                .into_iter()
                .find_map(program_on_path)
            {
                return Some(path);
            }
            if let Some(path) = versioned_lldb_dap() {
                return Some(path);
            }
            if !cfg!(target_os = "macos") {
                return None;
            }
            let output = Command::new("xcrun")
                .args(["-f", "lldb-dap"])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .ok()
                .filter(|output| output.status.success())?;
            let path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
            path.is_file().then_some(path)
        })
        .clone()
}

/// The newest `lldb-dap-18`-style program on the `PATH`, as Debian and
/// Ubuntu name LLVM's tools.
fn versioned_lldb_dap() -> Option<PathBuf> {
    std::env::split_paths(jig_tools::search_path())
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().into_string().ok()?;
            let version = name
                .strip_prefix("lldb-dap-")
                .or_else(|| name.strip_prefix("lldb-vscode-"))?
                .parse::<u32>()
                .ok()?;
            Some((version, entry.path()))
        })
        .max_by_key(|(version, _)| *version)
        .map(|(_, path)| path)
}

/// The unpacked `js-debug` folder: where Jig keeps debuggers, or Zed's.
fn js_debug() -> Option<PathBuf> {
    let is_js_debug = |dir: &Path| dir.join("src/dapDebugServer.js").is_file();
    if let Some(dir) = debuggers_dir()
        .map(|dir| dir.join("js-debug"))
        .filter(|dir| is_js_debug(dir))
    {
        return Some(dir);
    }
    let zed = data_dir()?
        .join(if cfg!(target_os = "macos") {
            "Zed"
        } else {
            "zed"
        })
        .join("debug_adapters/JavaScript");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(zed)
        .ok()?
        .filter_map(|entry| entry.ok().map(|entry| entry.path().join("js-debug")))
        .filter(|dir| is_js_debug(dir))
        .collect();
    versions.sort();
    versions.pop()
}

/// LLDB commands that load Rust's pretty-printers, so a `Vec` or `String`
/// shows its contents. Empty when there's no Rust toolchain.
fn rust_init_commands(cwd: &Path) -> Vec<String> {
    let Some(sysroot) = Command::new("rustc")
        .no_window()
        .args(["--print", "sysroot"])
        .current_dir(cwd)
        .env("PATH", jig_tools::search_path())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
    else {
        return Vec::new();
    };
    // Recent toolchains register everything from `lldb_lookup.py`; older
    // ones also have a commands file to source.
    let etc = sysroot.join("lib/rustlib/etc");
    let lookup = etc.join("lldb_lookup.py");
    if !lookup.is_file() {
        return Vec::new();
    }
    let mut commands = vec![format!("command script import \"{}\"", lookup.display())];
    let extra = etc.join("lldb_commands");
    if extra.is_file() {
        commands.push(format!("command source -s 0 \"{}\"", extra.display()));
    }
    commands.push(WITHOUT_STRUCT_SUMMARIES.into());
    commands
}

fn program_on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(jig_tools::search_path())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}
