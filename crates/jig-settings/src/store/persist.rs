//! The settings files in `~/.config/jig/`: `settings.toml`, `recent.toml`
//! and `sessions.toml`, read and written whole.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::domain::recent::RecentProjects;
use crate::domain::session::{Session, Sessions};
use crate::domain::settings::{MAX_FONT_SIZE, MAX_TRANSLUCENCY, MIN_FONT_SIZE, Settings};

impl Settings {
    /// `~/.config/jig/settings.toml`, honouring `XDG_CONFIG_HOME`.
    pub fn user_path() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        Some(config.join("jig").join("settings.toml"))
    }

    /// The saved settings, or the defaults when there is no file yet.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let source =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut settings: Settings =
            toml::from_str(&source).with_context(|| format!("parsing {}", path.display()))?;
        settings.appearance.font_size = settings
            .appearance
            .font_size
            .clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        settings.appearance.translucency =
            settings.appearance.translucency.clamp(0., MAX_TRANSLUCENCY);
        Ok(settings)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let source = toml::to_string_pretty(self)?;
        fs::write(path, source).with_context(|| format!("writing {}", path.display()))
    }
}

impl RecentProjects {
    /// Next to `settings.toml`.
    pub fn user_path() -> Option<PathBuf> {
        Some(Settings::user_path()?.parent()?.join("recent.toml"))
    }

    /// The saved list without folders that no longer exist; empty when
    /// there is no file yet.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let source =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut recent: Self =
            toml::from_str(&source).with_context(|| format!("parsing {}", path.display()))?;
        recent.projects.retain(|project| project.is_dir());
        Ok(recent)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let source = toml::to_string_pretty(self)?;
        fs::write(path, source).with_context(|| format!("writing {}", path.display()))
    }
}

impl Session {
    /// The files as full paths, leaving out those that no longer exist,
    /// and the index of the active one among them.
    pub fn existing_files(&self) -> (Vec<PathBuf>, usize) {
        let mut active = 0;
        let mut files = Vec::new();
        for (ix, file) in self.files.iter().enumerate() {
            let path = self.root.join(file);
            if !path.is_file() {
                continue;
            }
            if ix <= self.active {
                active = files.len();
            }
            files.push(path);
        }
        (files, active)
    }
}

impl Sessions {
    /// Next to `settings.toml`.
    pub fn user_path() -> Option<PathBuf> {
        Some(Settings::user_path()?.parent()?.join("sessions.toml"))
    }

    /// The saved sessions; none when there is no file yet.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let source =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&source).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let source = toml::to_string_pretty(self)?;
        fs::write(path, source).with_context(|| format!("writing {}", path.display()))
    }
}

#[cfg(test)]
mod tests;
