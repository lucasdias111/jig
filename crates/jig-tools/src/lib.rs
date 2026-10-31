//! Installing the tools Jig runs, as Neovim's Mason does: language servers,
//! debuggers and formatters each describe an `install` shell command, which
//! runs here into a folder of Jig's, where Jig then finds what it put. Tools
//! installed some other way (rustup, Homebrew, Xcode) are found on the PATH
//! and left alone.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use jig_process::NoWindow as _;
use serde::Deserialize;

/// How to get a tool: one text, or one per system.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
pub enum Help {
    Text(String),
    PerOs {
        macos: Option<String>,
        linux: Option<String>,
        default: Option<String>,
    },
}

impl Help {
    /// The text for this system.
    pub fn text(&self) -> String {
        match self {
            Help::Text(text) => text.clone(),
            Help::PerOs {
                macos,
                linux,
                default,
            } => {
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
}

/// The folder Jig keeps one kind of tool in (`servers`, `debuggers`,
/// `formatters`): Application Support on macOS, the XDG data folder
/// elsewhere.
pub fn dir(kind: &str) -> Option<PathBuf> {
    data_dir().map(|dir| {
        dir.join(if cfg!(target_os = "macos") {
            "Jig"
        } else {
            "jig"
        })
        .join(kind)
    })
}

/// `script` with `${os}` and `${arch}` filled in, as Rust names them
/// (`macos`, `linux`, `windows`; `aarch64`, `x86_64`), for installers
/// that download a build for this computer.
pub fn expand_platform(script: &str) -> String {
    script
        .replace("${os}", std::env::consts::OS)
        .replace("${arch}", std::env::consts::ARCH)
}

/// How long an installer gets. The biggest (jdtls, an LLVM build) are a few
/// hundred megabytes; past this, a download has stalled.
const INSTALL_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Run an installer, a shell script with Jig's variables already filled
/// in, in `dir`. Blocks, so run it off the main thread.
pub fn run_installer(script: &str, dir: &Path) -> Result<()> {
    run_installer_within(script, dir, INSTALL_TIMEOUT)
}

fn run_installer_within(script: &str, dir: &Path, limit: Duration) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
    let script = expand_platform(script);
    // Installers are sh scripts everywhere; on Windows, Git's sh runs them.
    let sh = if cfg!(windows) { "sh" } else { "/bin/sh" };
    let mut command = Command::new(sh);
    command
        .no_window()
        .arg("-c")
        .arg(&script)
        .current_dir(dir)
        .env("PATH", search_path())
        .stdin(Stdio::null());
    // A group of its own, so a stalled one stops with what it started.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let output = match jig_process::output_within(&mut command, limit) {
        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => bail!(
            "Installing took over {}, so it was stopped.",
            jig_process::describe(limit)
        ),
        result => result.context("Couldn't run the installer")?,
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last: Vec<&str> = stderr.trim().lines().rev().take(3).collect();
        let last: Vec<&str> = last.into_iter().rev().collect();
        bail!("Installing failed: {}", last.join(" "));
    }
    Ok(())
}

/// Run a tool's installer in a folder of its own, removing the folder
/// again if the installer left nothing in it (as rustup's doesn't, or one
/// that failed), so an empty folder never reads as installed.
pub fn install_own_folder(script: &str, dir: &Path) -> Result<()> {
    let script = script.replace("${dir}", &dir.to_string_lossy());
    let result = run_installer(&script, dir);
    if std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none()) {
        std::fs::remove_dir(dir).ok();
    }
    result
}

/// Where a tool installed into its own folder keeps its programs.
pub fn bin_dirs(own: &Path) -> [PathBuf; 2] {
    [own.join("bin"), own.join("node_modules/.bin")]
}

/// `name`, as an executable in `dir`. Windows adds an extension.
pub fn executable_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let extensions: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    extensions
        .iter()
        .map(|extension| dir.join(format!("{name}{extension}")))
        .find(|candidate| candidate.is_file())
}

/// `path` with the home folder as `~`.
pub fn home_relative(path: &Path) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match home
        .as_deref()
        .filter(|home| !home.as_os_str().is_empty())
        .and_then(|home| path.strip_prefix(home).ok())
    {
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// Application Support on macOS, the XDG data folder elsewhere.
pub fn data_dir() -> Option<PathBuf> {
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

/// Where Jig keeps debuggers it installs: Application Support on macOS,
/// the XDG data folder elsewhere. `JIG_DEBUGGERS_DIR` says otherwise.
pub fn debuggers_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("JIG_DEBUGGERS_DIR") {
        return Some(PathBuf::from(dir));
    }
    dir("debuggers")
}

/// Where Jig keeps servers it installs, beside its debuggers and
/// formatters. `JIG_SERVERS_DIR` says otherwise.
pub fn servers_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("JIG_SERVERS_DIR") {
        return Some(PathBuf::from(dir));
    }
    debuggers_dir()?.parent().map(|dir| dir.join("servers"))
}

/// `name` in the user's Jig config folder (`~/.config/jig`); `None` in
/// tests, here and the app's (the `testing` feature), so they never read the
/// user's own files.
pub fn user_file(name: &str) -> Option<PathBuf> {
    if cfg!(any(test, feature = "testing")) {
        return None;
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("jig").join(name))
}

/// The `PATH` servers and run configurations are found on and run with. An
/// app opened from the Finder gets only the system's minimal one, so ask the
/// login shell, as a terminal would have it, and add where installers
/// usually put things.
pub fn search_path() -> &'static OsString {
    static PATH: OnceLock<OsString> = OnceLock::new();
    PATH.get_or_init(|| {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(shell) = std::env::var_os("SHELL")
            && let Ok(output) = Command::new(shell)
                .args(["-l", "-c", "printf '%s' \"$PATH\""])
                .stdin(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .output()
            && output.status.success()
        {
            let shell_path = OsString::from(String::from_utf8_lossy(&output.stdout).trim());
            dirs.extend(std::env::split_paths(&shell_path));
        }
        if let Some(path) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&path));
        }
        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            for dir in [
                ".cargo/bin",
                ".local/bin",
                "go/bin",
                ".npm-global/bin",
                ".bun/bin",
                // OpenCode's installer, which adds it to the PATH only in
                // `.zshrc` and the like.
                ".opencode/bin",
            ] {
                dirs.push(home.join(dir));
            }
            dirs.extend(node_dirs(&home));
        }
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
        let mut seen = std::collections::HashSet::new();
        dirs.retain(|dir| seen.insert(dir.clone()));
        std::env::join_paths(dirs).unwrap_or_default()
    })
}

/// Where Node version managers keep `node` and `npm`. They set the `PATH`
/// up in `.zshrc` and the like, which a login shell doesn't read.
fn node_dirs(home: &Path) -> Vec<PathBuf> {
    let nvm = std::env::var_os("NVM_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".nvm"));
    let mut dirs: Vec<PathBuf> = nvm_bin(&nvm).into_iter().collect();
    dirs.extend(
        [
            ".volta/bin",
            ".local/share/fnm/aliases/default/bin",
            "Library/Application Support/fnm/aliases/default/bin",
            ".asdf/shims",
            ".local/share/mise/shims",
            "Library/pnpm",
            ".local/share/pnpm",
        ]
        .map(|dir| home.join(dir)),
    );
    dirs
}

/// nvm's default Node, by its `alias/default` ("24", "v24.1.0"), or the
/// newest one installed.
fn nvm_bin(nvm: &Path) -> Option<PathBuf> {
    let versions = nvm.join("versions/node");
    let mut installed: Vec<(Vec<u64>, String)> = std::fs::read_dir(&versions)
        .ok()?
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            let numbers = name
                .trim_start_matches('v')
                .split('.')
                .map(|part| part.parse().ok())
                .collect::<Option<Vec<u64>>>()?;
            Some((numbers, name))
        })
        .collect();
    installed.sort();
    let alias = std::fs::read_to_string(nvm.join("alias/default")).unwrap_or_default();
    let alias = alias.trim().trim_start_matches('v');
    let chosen = installed
        .iter()
        .rev()
        .find(|(_, name)| {
            let name = name.trim_start_matches('v');
            !alias.is_empty() && (name == alias || name.starts_with(&format!("{alias}.")))
        })
        .or(installed.last())?;
    Some(versions.join(&chosen.1).join("bin"))
}

pub fn find_program(name: &str, path: &OsString) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_variables_name_this_computer() {
        let script = expand_platform("get ${os}-${arch} into ${dir}");
        assert!(!script.contains("${os}") && !script.contains("${arch}"));
        assert!(script.ends_with("into ${dir}"), "others are left alone");
    }

    #[cfg(unix)]
    #[test]
    fn an_installer_that_puts_nothing_leaves_no_folder() {
        let parent = tempfile::tempdir().unwrap();
        let own = parent.path().join("tool");
        install_own_folder("true", &own).unwrap();
        assert!(!own.exists());
        install_own_folder("mkdir -p \"${dir}/bin\" && touch \"${dir}/bin/t\"", &own).unwrap();
        assert!(own.join("bin/t").is_file());
        let error = install_own_folder("echo nope >&2; exit 3", &parent.path().join("x"));
        assert_eq!(error.unwrap_err().to_string(), "Installing failed: nope");
    }

    #[cfg(unix)]
    #[test]
    fn a_stalled_installer_is_stopped() {
        let dir = tempfile::tempdir().unwrap();
        let error = run_installer_within("sleep 30", dir.path(), Duration::from_secs(1));
        assert_eq!(
            error.unwrap_err().to_string(),
            "Installing took over 1 s, so it was stopped."
        );
    }

    // Windows has no HOME, so paths there show in full.
    #[cfg(unix)]
    #[test]
    fn home_folder_shows_as_a_tilde() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(home_relative(&home.join(".cargo/bin/x")), "~/.cargo/bin/x");
        assert_eq!(home_relative(Path::new("/usr/bin/x")), "/usr/bin/x");
    }
    #[test]
    fn finds_nvm_s_default_node() {
        let nvm = tempfile::tempdir().unwrap();
        for version in ["v18.20.0", "v24.2.0", "v24.19.0", "v9.0.0"] {
            std::fs::create_dir_all(nvm.path().join("versions/node").join(version).join("bin"))
                .unwrap();
        }
        let bin = |version: &str| nvm.path().join("versions/node").join(version).join("bin");
        assert_eq!(nvm_bin(nvm.path()), Some(bin("v24.19.0")), "the newest");
        std::fs::create_dir_all(nvm.path().join("alias")).unwrap();
        std::fs::write(nvm.path().join("alias/default"), "18\n").unwrap();
        assert_eq!(nvm_bin(nvm.path()), Some(bin("v18.20.0")), "the default");
        std::fs::write(nvm.path().join("alias/default"), "v24.2.0").unwrap();
        assert_eq!(nvm_bin(nvm.path()), Some(bin("v24.2.0")));
        assert_eq!(nvm_bin(&nvm.path().join("nowhere")), None);
    }
}
