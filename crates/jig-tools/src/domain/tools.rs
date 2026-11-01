//! What a tool says about itself, and the paths around it, worked out
//! without looking at the disk.

use std::path::{Path, PathBuf};

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

/// `script` with `${os}` and `${arch}` filled in, as Rust names them
/// (`macos`, `linux`, `windows`; `aarch64`, `x86_64`), for installers
/// that download a build for this computer.
pub fn expand_platform(script: &str) -> String {
    script
        .replace("${os}", std::env::consts::OS)
        .replace("${arch}", std::env::consts::ARCH)
}

/// Where a tool installed into its own folder keeps its programs.
pub fn bin_dirs(own: &Path) -> [PathBuf; 2] {
    [own.join("bin"), own.join("node_modules/.bin")]
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

    // Windows has no HOME, so paths there show in full.
    #[cfg(unix)]
    #[test]
    fn home_folder_shows_as_a_tilde() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        assert_eq!(home_relative(&home.join(".cargo/bin/x")), "~/.cargo/bin/x");
        assert_eq!(home_relative(Path::new("/usr/bin/x")), "/usr/bin/x");
    }
}
