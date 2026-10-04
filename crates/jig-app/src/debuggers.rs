//! The debuggers Jig knows: what each debugs, how to find its adapter, and
//! what to say when it isn't there. Settings turns each on or off, and can
//! install js-debug, a download of plain JavaScript. LLDB comes from Xcode
//! on macOS and from the package manager on Linux, so Jig only finds it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

use anyhow::{Context as _, Result, bail};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// `lldb-dap`: Xcode's on macOS, LLVM's on Linux.
    Lldb,
    /// Microsoft's `js-debug`, run on Node.
    JsDebug,
}

pub struct Debugger {
    pub kind: Kind,
    /// Its key in `settings.toml`.
    pub key: &'static str,
    pub label: &'static str,
    /// The languages, by highlighter name, whose files take breakpoints.
    pub languages: &'static [&'static str],
}

pub const ALL: &[Debugger] = &[
    Debugger {
        kind: Kind::Lldb,
        key: "rust",
        label: "Rust, C and C++",
        languages: &["rust"],
    },
    Debugger {
        kind: Kind::JsDebug,
        key: "typescript",
        label: "TypeScript and JavaScript",
        languages: &["typescript", "tsx", "javascript"],
    },
];

/// The js-debug release Settings installs: the one Jig is tested with.
pub const JS_DEBUG_VERSION: &str = "1.140.0";

impl Debugger {
    /// Whether Settings can install it.
    pub fn installable(&self) -> bool {
        self.kind == Kind::JsDebug
    }

    /// How to get it, for Settings and for when debugging can't start.
    pub fn install_help(&self) -> String {
        match self.kind {
            Kind::Lldb if cfg!(target_os = "macos") => {
                "It comes with Xcode's command line tools: xcode-select --install".into()
            }
            Kind::Lldb => "Install LLDB with your package manager, for example \
                           sudo apt install lldb or sudo dnf install lldb; Jig finds \
                           lldb-dap (or lldb-dap-18 and so on) on the PATH."
                .into(),
            Kind::JsDebug => "Install it in Settings, under Languages, or give its folder there. \
                 It runs on Node, which needs installing too."
                .into(),
        }
    }
}

pub fn get(kind: Kind) -> &'static Debugger {
    ALL.iter()
        .find(|debugger| debugger.kind == kind)
        .expect("every kind is listed")
}

/// The debugger for files in `language`, if there is one.
pub fn for_language(language: &str) -> Option<&'static Debugger> {
    ALL.iter()
        .find(|debugger| debugger.languages.contains(&language))
}

/// A found adapter, and how it's run.
#[derive(Clone, Debug, PartialEq)]
pub enum Adapter {
    /// A program speaking DAP on its stdin and stdout.
    Stdio(PathBuf),
    /// A script Node runs as a DAP server on a TCP port.
    NodeServer { node: PathBuf, script: PathBuf },
}

impl Adapter {
    /// Where it is, for Settings.
    pub fn describe(&self) -> String {
        match self {
            Adapter::Stdio(path) => path.display().to_string(),
            Adapter::NodeServer { script, .. } => {
                // `…/js-debug/src/dapDebugServer.js` reads best as its folder.
                let folder = script.parent().and_then(Path::parent).unwrap_or(script);
                folder.display().to_string()
            }
        }
    }
}

/// What Settings says about `debugger`: where its adapter is, or what's
/// missing and how to get it.
pub fn status(debugger: &Debugger, path: Option<&str>) -> String {
    match find(debugger.kind, path) {
        Ok(adapter) => format!("Using {}", home_relative(&adapter.describe())),
        Err(missing) if debugger.installable() => {
            format!("{missing} Install it below, or give its folder under Debugger locations.")
        }
        Err(missing) => format!("{missing} {}", debugger.install_help()),
    }
}

/// `kind`'s adapter: at `path` when the user gave one, otherwise where it
/// usually is. Otherwise, what's missing.
pub fn find(kind: Kind, path: Option<&str>) -> Result<Adapter, String> {
    let given = path
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(expand_home);
    match kind {
        Kind::Lldb => match given {
            Some(path) if path.is_file() => Ok(Adapter::Stdio(path)),
            Some(path) => Err(format!("Nothing at {}.", path.display())),
            None => lldb_dap()
                .map(Adapter::Stdio)
                .ok_or_else(|| "lldb-dap isn't installed.".into()),
        },
        Kind::JsDebug => {
            let script = match given {
                Some(path) => js_debug_script(&path)
                    .ok_or_else(|| format!("No js-debug at {}.", path.display()))?,
                None => default_js_debug().ok_or("js-debug isn't installed.")?,
            };
            let node = program_on_path("node").ok_or("Node isn't installed.")?;
            Ok(Adapter::NodeServer { node, script })
        }
    }
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
/// the XDG data folder elsewhere.
pub fn debuggers_dir() -> Option<PathBuf> {
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

/// js-debug's server script, given the script itself, the unpacked
/// `js-debug` folder, or the folder it was unpacked into.
fn js_debug_script(path: &Path) -> Option<PathBuf> {
    [
        path.to_path_buf(),
        path.join("src/dapDebugServer.js"),
        path.join("js-debug/src/dapDebugServer.js"),
    ]
    .into_iter()
    .find(|candidate| {
        candidate.is_file()
            && candidate
                .file_name()
                .is_some_and(|name| name == "dapDebugServer.js")
    })
}

/// js-debug where Jig keeps debuggers, or where Zed keeps its copy.
fn default_js_debug() -> Option<PathBuf> {
    if let Some(script) = debuggers_dir().and_then(|dir| js_debug_script(&dir)) {
        return Some(script);
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
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    versions.sort();
    versions.iter().rev().find_map(|dir| js_debug_script(dir))
}

/// Download js-debug and unpack it where Jig keeps debuggers, replacing
/// any copy there. Blocks for the download, so run it off the main thread.
pub fn install_js_debug() -> Result<PathBuf> {
    let dir = debuggers_dir().context("No home folder to install into")?;
    let url = format!(
        "https://github.com/microsoft/vscode-js-debug/releases/download/\
         v{JS_DEBUG_VERSION}/js-debug-dap-v{JS_DEBUG_VERSION}.tar.gz"
    );
    install_js_debug_from(&url, &dir)
}

/// Unpack the js-debug archive at `url` into `dir`. A failure leaves any
/// copy already there as it was.
fn install_js_debug_from(url: &str, dir: &Path) -> Result<PathBuf> {
    let staging = dir.join(".js-debug-download");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging)
        .with_context(|| format!("Couldn't create {}", staging.display()))?;
    let result = (|| {
        let archive = staging.join("js-debug.tar.gz");
        run(Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--output",
            ])
            .arg(&archive)
            .arg(url))
        .context("Couldn't download js-debug")?;
        run(Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&staging))
        .context("Couldn't unpack js-debug")?;
        let unpacked = staging.join("js-debug");
        if js_debug_script(&unpacked).is_none() {
            bail!("The download wasn't js-debug");
        }
        let target = dir.join("js-debug");
        let _ = std::fs::remove_dir_all(&target);
        std::fs::rename(&unpacked, &target)
            .with_context(|| format!("Couldn't move js-debug into {}", dir.display()))?;
        js_debug_script(&target).context("js-debug went missing")
    })();
    let _ = std::fs::remove_dir_all(&staging);
    result
}

/// Run `command`, failing with what it printed to stderr.
fn run(command: &mut Command) -> Result<()> {
    let output = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("Couldn't run it")?;
    if !output.status.success() {
        bail!("{}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

fn program_on_path(name: &str) -> Option<PathBuf> {
    std::env::split_paths(crate::lsp::search_path())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

/// `path` with the home folder as `~`, as people write it.
fn home_relative(path: &str) -> String {
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn js_debug_is_found_from_any_of_its_folders() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("js-debug/src/dapDebugServer.js");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(&script, "").unwrap();
        for given in [
            dir.path().to_path_buf(),
            dir.path().join("js-debug"),
            script.clone(),
        ] {
            assert_eq!(js_debug_script(&given), Some(script.clone()), "{given:?}");
        }
        assert_eq!(js_debug_script(&dir.path().join("elsewhere")), None);
    }

    #[test]
    fn a_given_path_that_is_wrong_says_so() {
        assert_eq!(
            find(Kind::Lldb, Some("/nowhere/lldb-dap")),
            Err("Nothing at /nowhere/lldb-dap.".into())
        );
        assert_eq!(
            find(Kind::JsDebug, Some("/nowhere")),
            Err("No js-debug at /nowhere.".into())
        );
    }

    #[test]
    fn languages_map_to_their_debugger() {
        assert_eq!(for_language("tsx").map(|d| d.kind), Some(Kind::JsDebug));
        assert_eq!(for_language("rust").map(|d| d.kind), Some(Kind::Lldb));
        assert!(for_language("toml").is_none());
    }

    #[test]
    fn installs_js_debug_from_its_archive_and_keeps_the_old_copy_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        // An archive laid out like js-debug-dap's.
        let source = dir.path().join("source");
        fs::create_dir_all(source.join("js-debug/src")).unwrap();
        fs::write(source.join("js-debug/src/dapDebugServer.js"), "// new").unwrap();
        let archive = dir.path().join("js-debug.tar.gz");
        let packed = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&source)
            .arg("js-debug")
            .status()
            .unwrap();
        assert!(packed.success());
        let debuggers = dir.path().join("debuggers");
        fs::create_dir_all(debuggers.join("js-debug/src")).unwrap();
        fs::write(debuggers.join("js-debug/src/dapDebugServer.js"), "// old").unwrap();

        let missing = format!("file://{}", dir.path().join("missing.tar.gz").display());
        assert!(install_js_debug_from(&missing, &debuggers).is_err());
        let script = debuggers.join("js-debug/src/dapDebugServer.js");
        assert_eq!(fs::read_to_string(&script).unwrap(), "// old");

        let url = format!("file://{}", archive.display());
        assert_eq!(install_js_debug_from(&url, &debuggers).unwrap(), script);
        assert_eq!(fs::read_to_string(&script).unwrap(), "// new");
        assert!(!debuggers.join(".js-debug-download").exists());
    }

    /// Downloads the real js-debug release into a temporary folder.
    #[test]
    #[ignore]
    fn install_js_debug_live() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!(
            "https://github.com/microsoft/vscode-js-debug/releases/download/\
             v{JS_DEBUG_VERSION}/js-debug-dap-v{JS_DEBUG_VERSION}.tar.gz"
        );
        let script = install_js_debug_from(&url, dir.path()).unwrap();
        assert!(script.ends_with("js-debug/src/dapDebugServer.js"));
        assert!(script.is_file());
    }
}
