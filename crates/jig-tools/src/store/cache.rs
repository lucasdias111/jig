//! The `PATH` programs are found on, asked of the login shell once and kept
//! for the life of the app.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

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

/// `name` in the first folder of `path` holding it.
pub fn find_program(name: &str, path: &OsString) -> Option<PathBuf> {
    std::env::split_paths(path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

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
