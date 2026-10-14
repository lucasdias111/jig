//! Installing the tools Jig runs, as Neovim's Mason does: language servers,
//! debuggers and formatters each describe an `install` shell command, which
//! runs here into a folder of Jig's, where Jig then finds what it put. Tools
//! installed some other way (rustup, Homebrew, Xcode) are found on the PATH
//! and left alone.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use jig_ai::process::NoWindow as _;
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
    crate::debuggers::data_dir().map(|dir| {
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

/// Run an installer, a shell script with Jig's variables already filled
/// in, in `dir`. Blocks, so run it off the main thread.
pub fn run_installer(script: &str, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
    let script = expand_platform(script);
    // Installers are sh scripts everywhere; on Windows, Git's sh runs them.
    let sh = if cfg!(windows) { "sh" } else { "/bin/sh" };
    let output = Command::new(sh)
        .no_window()
        .arg("-c")
        .arg(&script)
        .current_dir(dir)
        .env("PATH", crate::lsp::search_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("Couldn't run the installer")?;
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

    // Windows has no HOME, so paths there show in full.
    #[cfg(unix)]
    #[test]
    fn home_folder_shows_as_a_tilde() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(home_relative(&home.join(".cargo/bin/x")), "~/.cargo/bin/x");
        assert_eq!(home_relative(Path::new("/usr/bin/x")), "/usr/bin/x");
    }
}
