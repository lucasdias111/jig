//! Which copy of Jig is running, and whether it can update itself: an app
//! bundle, an executable, or a build from source.

use std::path::{Path, PathBuf};

/// What gets replaced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installation {
    /// macOS: the whole `Jig.app`.
    App(PathBuf),
    /// Linux and Windows: the executable.
    Executable(PathBuf),
}

impl Installation {
    /// The folder the new build is unpacked in and moved from.
    pub fn folder(&self) -> Option<&Path> {
        match self {
            Installation::App(path) | Installation::Executable(path) => path.parent(),
        }
    }
}

/// Which installation `exe` belongs to (`macos`: in an app bundle), or
/// why it can't update itself.
pub fn locate(exe: &Path, macos: bool) -> Result<Installation, String> {
    let built_here = "This is a build from source; update it with git and cargo.";
    if is_cargo_build(exe) {
        return Err(built_here.into());
    }
    if !macos {
        return Ok(Installation::Executable(exe.to_path_buf()));
    }
    // macOS runs an app opened straight from a download from a read-only
    // copy somewhere private.
    if exe
        .components()
        .any(|part| part.as_os_str() == "AppTranslocation")
    {
        return Err(
            "Move Jig to your Applications folder, then open it from there to update it.".into(),
        );
    }
    let bundle = exe
        .ancestors()
        .nth(3)
        .filter(|bundle| bundle.extension().is_some_and(|ext| ext == "app"))
        .filter(|bundle| exe.starts_with(bundle.join("Contents/MacOS")));
    match bundle {
        Some(bundle) => Ok(Installation::App(bundle.to_path_buf())),
        None => Err(built_here.into()),
    }
}

/// Whether `exe` is where cargo builds it, `target/<profile>/`.
pub fn is_cargo_build(exe: &Path) -> bool {
    let parts: Vec<_> = exe.components().map(|part| part.as_os_str()).collect();
    parts
        .iter()
        .rev()
        .skip(1)
        .take(3)
        .any(|part| *part == "target")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_app_bundle_or_the_executable() {
        assert_eq!(
            locate(Path::new("/Applications/Jig.app/Contents/MacOS/Jig"), true),
            Ok(Installation::App("/Applications/Jig.app".into()))
        );
        assert_eq!(
            locate(Path::new("/home/me/.local/bin/Jig"), false),
            Ok(Installation::Executable("/home/me/.local/bin/Jig".into()))
        );
        for built in [
            "/src/jig/target/release/Jig",
            "/src/jig/target/debug/Jig",
            "/src/jig/target/aarch64-apple-darwin/release/Jig",
        ] {
            assert!(locate(Path::new(built), false).is_err(), "{built}");
        }
        assert!(
            locate(Path::new("/usr/local/bin/Jig"), true).is_err(),
            "not in a bundle"
        );
        assert!(
            locate(
                Path::new("/private/var/folders/x/AppTranslocation/1/d/Jig.app/Contents/MacOS/Jig"),
                true
            )
            .unwrap_err()
            .contains("Applications")
        );
    }
}
