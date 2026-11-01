//! Jig's folders on disk, and running installers into them.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use jig_process::NoWindow as _;

use super::cache::search_path;
use crate::domain::tools::expand_platform;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
