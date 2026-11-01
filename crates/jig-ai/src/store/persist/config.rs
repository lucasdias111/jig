//! `config.toml` on disk: where it is, reading it, and starting it from the
//! built-in config.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use toml_edit::DocumentMut;

use crate::domain::config::{CONFIG_HEADER, Config, DEFAULT_CONFIG};
use crate::domain::config_file::ConfigFile;

impl Config {
    /// `~/.config/jig/config.toml`, honouring `XDG_CONFIG_HOME`.
    pub fn user_path() -> Option<PathBuf> {
        Some(config_dir()?.join("config.toml"))
    }

    /// The user's config if it exists, otherwise the built-in one.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        match path.filter(|path| path.exists()) {
            Some(path) => {
                let source = std::fs::read_to_string(path)
                    .with_context(|| format!("reading {}", path.display()))?;
                Self::parse(&source).with_context(|| format!("parsing {}", path.display()))
            }
            None => Ok(Self::built_in()),
        }
    }

    /// Write the built-in config to `path` if there is nothing there yet,
    /// so it can be edited.
    pub fn ensure_user_file(path: &Path) -> Result<()> {
        if path.exists() {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let source = format!("{CONFIG_HEADER}{}", DEFAULT_CONFIG.trim_start());
        std::fs::write(path, source).with_context(|| format!("writing {}", path.display()))
    }
}

impl ConfigFile {
    /// Open the file, writing the built-in providers there first if it
    /// doesn't exist yet.
    pub fn open(path: &Path) -> Result<Self> {
        Config::ensure_user_file(path)?;
        let source =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Config::parse(&source).with_context(|| format!("parsing {}", path.display()))?;
        let doc = source
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            doc,
        })
    }

    /// Write the file and return what it now says. Refuses to write
    /// anything Jig couldn't read back.
    pub fn save(&self) -> Result<Config> {
        let source = self.doc.to_string();
        let config = Config::parse(&source)?;
        std::fs::write(&self.path, source)
            .with_context(|| format!("writing {}", self.path.display()))?;
        Ok(config)
    }
}

/// Jig's config folder, `~/.config/jig`, honouring `XDG_CONFIG_HOME`.
pub(crate) fn config_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("jig"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_file_starts_as_the_built_in_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jig").join("config.toml");
        Config::ensure_user_file(&path).unwrap();
        let config = Config::load(Some(&path)).unwrap();
        assert_eq!(config.providers.len(), Config::built_in().providers.len());
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with("# Jig's AI")
        );

        // An existing file is left alone.
        std::fs::write(&path, "mine").unwrap();
        Config::ensure_user_file(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
    }
}
