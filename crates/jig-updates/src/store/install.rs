//! Putting a downloaded build in place of the one running, and starting it.
//!
//! The new build is unpacked beside the old one, so moving it in is a
//! rename on the same disk; the old one is moved aside first and moved
//! back if that fails. A running program can be renamed everywhere (on
//! Windows, though not deleted, so its old `.exe` is cleared up at the next
//! start).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, anyhow, bail};
use jig_process::NoWindow as _;

use crate::domain::install::{Installation, locate};

/// This copy of Jig, or why it can't update itself.
pub fn installation() -> Result<Installation, String> {
    let exe = std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .map_err(|error| format!("Jig couldn't tell where it's installed: {error}"))?;
    locate(&exe, cfg!(target_os = "macos"))
}

/// Replace `installation` with the build in `archive`. Returns what to
/// start.
pub fn install(installation: &Installation, archive: &Path) -> Result<PathBuf> {
    let folder = installation
        .folder()
        .ok_or_else(|| anyhow!("Jig is installed at the top of the disk."))?;
    let staging = folder.join(format!(".jig-update-{}", std::process::id()));
    remove(&staging);
    fs::create_dir(&staging).map_err(|_| {
        anyhow!(
            "Jig can't write to {}, so it can't update itself. Download the new version from \
             GitHub instead.",
            folder.display()
        )
    })?;
    let installed = unpack(archive, &staging).and_then(|()| match installation {
        Installation::App(bundle) => swap(&staging.join("Jig.app"), bundle, |new| {
            new.join("Contents/MacOS/Jig").is_file()
        }),
        Installation::Executable(exe) => {
            let name = exe.file_name().unwrap_or_default();
            swap(&staging.join(name), exe, |new| new.is_file())
        }
    });
    remove(&staging);
    installed?;
    let installed = match installation {
        Installation::App(path) | Installation::Executable(path) => path.clone(),
    };
    Ok(installed)
}

/// Move `new` to `old`'s place, `old` aside and then away. On Windows the
/// running `.exe` can't be deleted, so it stays beside the new one as
/// `Jig.exe.old` until [`clean_up`].
fn swap(new: &Path, old: &Path, is_build: impl Fn(&Path) -> bool) -> Result<()> {
    if !is_build(new) {
        bail!("The update doesn't hold a build of Jig.");
    }
    #[cfg(unix)]
    if new.is_file() {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(new, fs::Permissions::from_mode(0o755))?;
    }
    let aside = set_aside_path(old);
    remove(&aside);
    fs::rename(old, &aside).with_context(|| format!("moving {} aside", old.display()))?;
    if let Err(error) = fs::rename(new, old) {
        // Put the old one back, so Jig still opens.
        let _ = fs::rename(&aside, old);
        return Err(error).with_context(|| format!("moving the update to {}", old.display()));
    }
    if !cfg!(windows) {
        remove(&aside);
    }
    Ok(())
}

/// `Jig.app` → `.Jig.app.old`, `Jig.exe` → `Jig.exe.old`: where the old
/// build waits while the new one moves in.
fn set_aside_path(old: &Path) -> PathBuf {
    let name = old.file_name().unwrap_or_default().to_string_lossy();
    let hidden = if cfg!(windows) { "" } else { "." };
    old.with_file_name(format!("{hidden}{name}.old"))
}

/// Clear up after an update: the old `.exe` Windows wouldn't let go of.
pub fn clean_up() {
    if cfg!(windows)
        && let Ok(Installation::Executable(exe)) = installation()
    {
        remove(&set_aside_path(&exe));
    }
}

/// Unpack `archive` into `into` with the system's own tools: `ditto` keeps
/// an app bundle's links and permissions, and `tar` reads zips on Windows
/// as well as `.tar.gz` everywhere.
fn unpack(archive: &Path, into: &Path) -> Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        let mut ditto = Command::new("ditto");
        ditto.arg("-x").arg("-k").arg(archive).arg(into);
        ditto
    } else {
        let mut tar = Command::new("tar");
        tar.arg("-xf").arg(archive).arg("-C").arg(into);
        tar
    };
    let output = command
        .no_window()
        .stdin(Stdio::null())
        .output()
        .context("running the unpacker")?;
    if !output.status.success() {
        bail!(
            "The update couldn't be unpacked: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

/// Start the installed Jig on `path` (the project or file that was open),
/// a new process that outlives this one.
pub fn relaunch(installed: &Path, path: Option<&Path>) -> Result<()> {
    let mut command = if cfg!(target_os = "macos") {
        // `-n`: a new instance, though this one is still quitting.
        let mut open = Command::new("open");
        open.arg("-n").arg(installed);
        if let Some(path) = path {
            open.arg("--args").arg(path);
        }
        open
    } else {
        let mut jig = Command::new(installed);
        jig.args(path);
        jig
    };
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("starting the new version")?;
    Ok(())
}

fn remove(path: &Path) {
    let _ = if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn replaces_the_executable_with_the_one_unpacked() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("Jig");
        fs::write(&exe, "old").unwrap();
        let build = dir.path().join("build");
        fs::create_dir(&build).unwrap();
        fs::write(build.join("Jig"), "new").unwrap();
        let archive = dir.path().join("Jig.tar.gz");
        let packed = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&build)
            .arg("Jig")
            .status()
            .unwrap();
        assert!(packed.success());

        let installation = Installation::Executable(exe.clone());
        // macOS unpacks with ditto, which wants a zip; tar is what the
        // other platforms use, so test the swap itself here.
        if !cfg!(target_os = "macos") {
            assert_eq!(install(&installation, &archive).unwrap(), exe);
        } else {
            swap(&build.join("Jig"), &exe, |new| new.is_file()).unwrap();
        }
        assert_eq!(fs::read_to_string(&exe).unwrap(), "new");
        let left: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(
            !left
                .iter()
                .any(|name| name.to_string_lossy().contains(".old")),
            "{left:?}"
        );
    }

    #[test]
    fn a_failed_swap_keeps_the_old_build() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("Jig");
        fs::write(&exe, "old").unwrap();
        assert!(swap(&dir.path().join("missing"), &exe, |new| new.is_file()).is_err());
        assert_eq!(fs::read_to_string(&exe).unwrap(), "old");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn replaces_the_app_bundle() {
        let dir = tempfile::tempdir().unwrap();
        let make = |root: &Path, text: &str| {
            let macos = root.join("Jig.app/Contents/MacOS");
            fs::create_dir_all(&macos).unwrap();
            fs::write(macos.join("Jig"), text).unwrap();
        };
        let apps = dir.path().join("Applications");
        make(&apps, "old");
        let build = dir.path().join("build");
        make(&build, "new");
        let archive = dir.path().join("Jig-macos-arm64.zip");
        let packed = Command::new("ditto")
            .args(["-c", "-k", "--keepParent"])
            .arg(build.join("Jig.app"))
            .arg(&archive)
            .status()
            .unwrap();
        assert!(packed.success());

        let bundle = apps.join("Jig.app");
        assert_eq!(
            install(&Installation::App(bundle.clone()), &archive).unwrap(),
            bundle
        );
        assert_eq!(
            fs::read_to_string(bundle.join("Contents/MacOS/Jig")).unwrap(),
            "new"
        );
        let left: Vec<_> = fs::read_dir(&apps)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(left, ["Jig.app"], "nothing left beside it");
    }
}
