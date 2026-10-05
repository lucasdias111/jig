//! The debuggers Jig knows, as described in `assets/default-debuggers.toml`
//! and the user's `~/.config/jig/debuggers.toml`: how to start each one's
//! adapter, which run configurations it takes, how to install it, and the
//! `launch` request to send, with `${variables}` filled in. Any debugger
//! speaking the Debug Adapter Protocol can be added there; nothing about a
//! particular one lives in code but a few tools Jig finds for you.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::{Value, json};

const BUILT_IN: &str = include_str!("../../../assets/default-debuggers.toml");

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
    adapter: Vec<String>,
    #[serde(default)]
    server: Vec<String>,
    /// Asks this language server, as Jig runs it, for the adapter's port.
    lsp: Option<String>,
    /// The `workspace/executeCommand` that answers with the port.
    lsp_start: Option<String>,
    /// Plugins the language server loads at start; `*` matches in a file
    /// name.
    #[serde(default)]
    lsp_bundles: Vec<String>,
    #[serde(default)]
    adapter_env: BTreeMap<String, String>,
    adapter_id: Option<String>,
    check: Option<Vec<String>>,
    install: Option<String>,
    help: Option<Help>,
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

#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum Help {
    Text(String),
    PerOs {
        macos: Option<String>,
        linux: Option<String>,
        default: Option<String>,
    },
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
            Some(Help::Text(text)) => text.clone(),
            Some(Help::PerOs {
                macos,
                linux,
                default,
            }) => {
                let specific = if cfg!(target_os = "macos") {
                    macos
                } else if cfg!(target_os = "linux") {
                    linux
                } else {
                    &None
                };
                specific
                    .clone()
                    .or_else(|| default.clone())
                    .unwrap_or_default()
            }
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

    /// The adapter to start, with variables filled in, or what's missing.
    pub fn adapter_command(&self, vars: &Vars) -> Result<AdapterCommand, String> {
        let (words, server) = if self.server.is_empty() {
            (&self.adapter, false)
        } else {
            (&self.server, true)
        };
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

    fn passes_check(&self, vars: &Vars) -> bool {
        let Some(check) = &self.check else {
            return true;
        };
        let mut checks = CHECKS.lock().unwrap();
        let checks = checks.get_or_insert_with(HashMap::new);
        if let Some(passed) = checks.get(&self.key) {
            return *passed;
        }
        let passed = (|| {
            let words = check
                .iter()
                .map(|word| vars.expand_str(word))
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            let env = self
                .adapter_env
                .iter()
                .map(|(key, value)| Some((key.clone(), vars.expand_str(value).ok()?)))
                .collect::<Option<Vec<_>>>()?;
            let status = Command::new(resolve_program(&words[0]).ok()?)
                .args(&words[1..])
                .envs(env)
                .env("PATH", crate::lsp::search_path())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .ok()?;
            Some(status.success())
        })()
        .unwrap_or(false);
        checks.insert(self.key.clone(), passed);
        passed
    }
}

/// Which `check`s passed, so Settings doesn't run them on every repaint.
static CHECKS: Mutex<Option<HashMap<String, bool>>> = Mutex::new(None);

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

/// The debuggers as they are now; the user's file is read again when it
/// has changed.
pub fn registry() -> Arc<Registry> {
    type Cached = (Option<SystemTime>, Arc<Registry>);
    static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
    let path = user_path();
    let modified = path
        .as_deref()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|meta| meta.modified().ok());
    let mut cache = CACHE.lock().unwrap();
    if let Some((stamp, registry)) = cache.as_ref()
        && *stamp == modified
    {
        return registry.clone();
    }
    let registry = Arc::new(load(path.as_deref()));
    *CHECKS.lock().unwrap() = None;
    *cache = Some((modified, registry.clone()));
    registry
}

/// The user's `debuggers.toml`; `None` in tests.
pub fn user_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/jig/debuggers.toml"))
}

fn parse(text: &str) -> Result<Vec<Debugger>> {
    let file: DebuggerFile = toml::from_str(text)?;
    for debugger in &file.debugger {
        debugger.validate()?;
    }
    Ok(file.debugger)
}

fn load(user: Option<&Path>) -> Registry {
    let mut debuggers = parse(BUILT_IN).expect("the built-in debuggers parse");
    let mut error = None;
    if let Some(path) = user
        && let Ok(text) = std::fs::read_to_string(path)
    {
        match parse(&text) {
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

/// Values for `${variables}`. Tools are looked for only when used.
pub struct Vars {
    values: HashMap<String, Value>,
    cwd: PathBuf,
}

impl Vars {
    pub fn new(cwd: &Path) -> Self {
        Self {
            values: HashMap::new(),
            cwd: cwd.to_path_buf(),
        }
    }

    pub fn set(&mut self, name: &str, value: Value) -> &mut Self {
        self.values.insert(name.to_string(), value);
        self
    }

    pub fn get(&self, name: &str) -> Result<Value, String> {
        if let Some(value) = self.values.get(name) {
            return Ok(value.clone());
        }
        let path = |path: Option<PathBuf>, missing: &str| {
            path.map(|path| json!(path))
                .ok_or_else(|| missing.to_string())
        };
        match name {
            "debuggers" => path(debuggers_dir(), "There's no home folder for debuggers."),
            "node" => path(program_on_path("node"), "Node isn't installed."),
            "lldb_dap" => path(lldb_dap(), "lldb-dap isn't installed."),
            "js_debug" => path(js_debug(), "js-debug isn't installed."),
            "lldb_rust_formatters" => Ok(json!(rust_init_commands(&self.cwd))),
            _ => Err(format!(
                "debuggers.toml uses ${{{name}}}, which Jig doesn't know."
            )),
        }
    }

    /// `text` with its variables filled in, as text.
    pub fn expand_str(&self, text: &str) -> Result<String, String> {
        let mut out = String::new();
        let mut rest = text;
        while let Some(start) = rest.find("${") {
            out.push_str(&rest[..start]);
            let after = &rest[start + 2..];
            let Some(end) = after.find('}') else {
                out.push_str(&rest[start..]);
                return Ok(out);
            };
            match self.get(&after[..end])? {
                Value::String(s) => out.push_str(&s),
                Value::Null => {}
                other => out.push_str(&other.to_string()),
            }
            rest = &after[end + 1..];
        }
        out.push_str(rest);
        Ok(out)
    }

    /// `value` with its variables filled in. A string that is just one
    /// variable becomes that variable's value, list or table as it is.
    pub fn expand(&self, value: &Value) -> Result<Value, String> {
        Ok(match value {
            Value::String(text) => {
                let whole = text
                    .strip_prefix("${")
                    .and_then(|t| t.strip_suffix('}'))
                    .filter(|name| !name.contains(['$', '{', '}']));
                match whole {
                    Some(name) => self.get(name)?,
                    None => Value::String(self.expand_str(text)?),
                }
            }
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .map(|item| self.expand(item))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(key, value)| Ok((key.clone(), self.expand(value)?)))
                    .collect::<Result<_, String>>()?,
            ),
            other => other.clone(),
        })
    }
}

/// The plugins debuggers ask language server `server` to load, that are
/// installed.
pub fn lsp_bundles(server: &str) -> Vec<PathBuf> {
    let vars = Vars::new(&std::env::temp_dir());
    registry()
        .debuggers
        .iter()
        .filter(|d| d.lsp.as_deref() == Some(server))
        .flat_map(|d| d.lsp_bundles.iter())
        .filter_map(|pattern| vars.expand_str(pattern).ok())
        .flat_map(|pattern| glob(&pattern))
        .collect()
}

/// The files matching `pattern`, whose last part may have one `*`.
fn glob(pattern: &str) -> Vec<PathBuf> {
    let path = PathBuf::from(pattern);
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
        return Vec::new();
    };
    let Some((before, after)) = name.split_once('*') else {
        return path.is_file().then_some(path.clone()).into_iter().collect();
    };
    let mut found: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.len() >= before.len() + after.len()
                && name.starts_with(before)
                && name.ends_with(after)
        })
        .map(|entry| entry.path())
        .collect();
    found.sort();
    found
}

/// What's missing for a debugger that lives in a language server.
fn lsp_missing(debugger: &Debugger, server: &str) -> Option<String> {
    if !crate::lsp::is_installed(server) {
        return Some(format!("{server} isn't installed."));
    }
    let vars = Vars::new(&std::env::temp_dir());
    let missing_plugin = debugger.lsp_bundles.iter().any(|pattern| {
        vars.expand_str(pattern)
            .map_or(true, |pattern| glob(&pattern).is_empty())
    });
    missing_plugin.then(|| {
        format!(
            "{}'s debugger plugin for {server} isn't installed.",
            debugger.name
        )
    })
}

/// Why `debugger` can't start, or `None` when it can.
pub fn missing(debugger: &Debugger, cwd: &Path) -> Option<String> {
    if let Some((server, _)) = debugger.lsp() {
        return lsp_missing(debugger, server);
    }
    let mut vars = Vars::new(cwd);
    vars.set("port", json!("0"));
    match debugger.adapter_command(&vars) {
        Err(what) => Some(what),
        Ok(_) if !debugger.passes_check(&vars) => {
            Some(format!("{}'s debugger isn't installed.", debugger.name))
        }
        Ok(_) => None,
    }
}

/// What Settings says about `debugger`: where its adapter is, or what's
/// missing, and whether it's ready.
pub fn status(debugger: &Debugger) -> (String, bool) {
    let cwd = std::env::temp_dir();
    if let Some(what) = missing(debugger, &cwd) {
        let how = if debugger.installable() {
            "Install it below.".to_string()
        } else {
            debugger.help()
        };
        return (format!("{what} {how}"), false);
    }
    if let Some((server, _)) = debugger.lsp() {
        let plugin = lsp_bundles(server)
            .into_iter()
            .next()
            .map(|path| home_relative(&path))
            .unwrap_or_default();
        return (format!("Using {server} with {plugin}"), true);
    }
    let mut vars = Vars::new(&cwd);
    vars.set("port", json!("0"));
    let shown = debugger
        .adapter_command(&vars)
        .map(|adapter| home_relative(&adapter.shown))
        .unwrap_or_default();
    (format!("Using {shown}"), true)
}

/// Run `debugger`'s install command. Blocks, so run it off the main thread.
pub fn install(debugger: &Debugger) -> Result<()> {
    let dir = debuggers_dir().context("No home folder to install into")?;
    install_into(debugger, &dir)
}

fn install_into(debugger: &Debugger, dir: &Path) -> Result<()> {
    let script = debugger.install.as_deref().context("Nothing to install")?;
    std::fs::create_dir_all(dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
    let mut vars = Vars::new(dir);
    vars.set("debuggers", json!(dir));
    let script = vars.expand_str(script).map_err(anyhow::Error::msg)?;
    let output = crate::run_output::shell_command(&script)
        .current_dir(dir)
        .env("PATH", crate::lsp::search_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("Couldn't run the installer")?;
    *CHECKS.lock().unwrap() = None;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last: Vec<&str> = stderr.trim().lines().rev().take(3).collect();
        let last: Vec<&str> = last.into_iter().rev().collect();
        bail!("Installing failed: {}", last.join(" "));
    }
    Ok(())
}

/// A program by name: on the `PATH`, or in `${debuggers}/bin`. A path must
/// exist.
fn resolve_program(word: &str) -> Result<PathBuf, String> {
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
    std::env::split_paths(crate::lsp::search_path())
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

/// Where Jig keeps debuggers it installs: Application Support on macOS,
/// the XDG data folder elsewhere. `JIG_DEBUGGERS_DIR` says otherwise.
pub fn debuggers_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("JIG_DEBUGGERS_DIR") {
        return Some(PathBuf::from(dir));
    }
    data_dir().map(|dir| {
        dir.join(if cfg!(target_os = "macos") {
            "Jig"
        } else {
            "jig"
        })
        .join("debuggers")
    })
}

fn data_dir() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var_os("HOME")?);
    if cfg!(target_os = "macos") {
        return Some(home.join("Library/Application Support"));
    }
    Some(
        std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .unwrap_or_else(|| home.join(".local/share")),
    )
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
        .args(["--print", "sysroot"])
        .current_dir(cwd)
        .env("PATH", crate::lsp::search_path())
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
    commands
}

fn program_on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(crate::lsp::search_path())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// `path` with the home folder as `~`, as people write it.
fn home_relative(path: &Path) -> String {
    let path = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn the_built_ins_parse_and_cover_five_languages() {
        let registry = load(None);
        assert!(registry.error.is_none());
        let keys: Vec<&str> = registry.debuggers.iter().map(|d| d.key.as_str()).collect();
        assert_eq!(keys, ["rust", "typescript", "python", "go", "java"]);
        assert_eq!(
            registry.get("java").unwrap().lsp(),
            Some(("jdtls", "vscode.java.startDebugSession"))
        );
        assert_eq!(registry.for_language("tsx").unwrap().key, "typescript");
        assert!(registry.for_language("toml").is_none());
    }

    #[test]
    fn the_users_file_replaces_and_adds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("debuggers.toml");
        fs::write(
            &path,
            r#"
[[debugger]]
key = "python"
name = "My Python"
adapter = ["mypy-dap"]
[debugger.launch]
request = "launch"

[[debugger]]
key = "ruby"
name = "Ruby"
languages = ["ruby"]
server = ["rdbg", "--port", "${port}"]
[debugger.launch]
request = "launch"
"#,
        )
        .unwrap();
        let registry = load(Some(&path));
        assert!(registry.error.is_none(), "{:?}", registry.error);
        assert_eq!(registry.get("python").unwrap().name, "My Python");
        assert_eq!(registry.get("ruby").unwrap().adapter_id(), "ruby");
        assert_eq!(registry.debuggers.len(), 6);

        fs::write(
            &path,
            "[[debugger]]\nkey = \"x\"\nname = \"X\"\n[debugger.launch]\n",
        )
        .unwrap();
        let broken = load(Some(&path));
        assert!(broken.error.unwrap().contains("adapter"));
        assert_eq!(broken.debuggers.len(), 5, "the built-ins stay");
    }

    #[test]
    fn variables_fill_in_text_and_keep_their_type_when_whole() {
        let mut vars = Vars::new(Path::new("/p"));
        vars.set("program", json!("/p/app.py"))
            .set("args", json!(["-v", "two words"]))
            .set("env", json!({"PORT": "8080"}));
        let template = json!({
            "program": "${program}",
            "args": "${args}",
            "env": "${env}",
            "title": "debugging ${program} now",
            "nested": ["${program}", {"a": "${args}"}],
            "plain": 3,
        });
        assert_eq!(
            vars.expand(&template).unwrap(),
            json!({
                "program": "/p/app.py",
                "args": ["-v", "two words"],
                "env": {"PORT": "8080"},
                "title": "debugging /p/app.py now",
                "nested": ["/p/app.py", {"a": ["-v", "two words"]}],
                "plain": 3,
            })
        );
        assert!(
            vars.expand(&json!("${nonsense}"))
                .unwrap_err()
                .contains("${nonsense}")
        );
    }

    #[test]
    fn bundles_match_by_file_name() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["plugin-0.53.1.jar", "plugin-0.52.0.jar", "other.jar"] {
            fs::write(dir.path().join(name), "").unwrap();
        }
        let pattern = format!("{}/plugin-*.jar", dir.path().display());
        assert_eq!(
            glob(&pattern),
            [
                dir.path().join("plugin-0.52.0.jar"),
                dir.path().join("plugin-0.53.1.jar")
            ]
        );
        assert!(glob(&format!("{}/missing-*.jar", dir.path().display())).is_empty());
    }

    #[test]
    fn help_follows_the_system() {
        let rust = load(None).get("rust").unwrap();
        let expected = if cfg!(target_os = "macos") {
            "xcode-select"
        } else {
            "package manager"
        };
        assert!(rust.help().contains(expected), "{}", rust.help());
    }

    #[test]
    fn install_runs_the_command_in_the_debuggers_folder() {
        let dir = tempfile::tempdir().unwrap();
        let mut debugger = load(None).get("go").unwrap().as_ref().clone();
        debugger.install =
            Some(r#"mkdir -p "${debuggers}/bin" && touch "${debuggers}/bin/made""#.into());
        install_into(&debugger, dir.path()).unwrap();
        assert!(dir.path().join("bin/made").is_file());

        debugger.install = Some("echo nope >&2; exit 3".into());
        let error = install_into(&debugger, dir.path()).unwrap_err().to_string();
        assert!(error.contains("nope"), "{error}");
    }

    /// Runs every built-in install command into a temporary folder, as
    /// Settings' Install button would. Downloads js-debug, debugpy, Delve,
    /// java-debug and, unless it's installed, jdtls; Go's module cache goes
    /// to the temporary folder too.
    #[test]
    #[ignore]
    fn built_in_installers_live() {
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: run on its own; nothing else reads these meanwhile.
        unsafe {
            std::env::set_var("GOPATH", dir.path().join("gopath"));
            std::env::set_var("GOFLAGS", "-modcacherw");
        }
        let debuggers = dir.path().join("debuggers");
        let registry = load(None);
        for (key, made) in [
            ("typescript", "js-debug/src/dapDebugServer.js"),
            ("python", "debugpy/debugpy/__init__.py"),
            ("go", "bin/dlv"),
            (
                "java",
                "java-debug/com.microsoft.java.debug.plugin-0.53.1.jar",
            ),
        ] {
            install_into(&registry.get(key).unwrap(), &debuggers)
                .unwrap_or_else(|e| panic!("{key}: {e:#}"));
            assert!(debuggers.join(made).is_file(), "{key} made no {made}");
        }
    }
}
