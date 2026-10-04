//! `~/.config/jig/recent.toml`: the folders most recently opened as
//! projects, newest first, for the home page.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};

/// How many the home page lists.
pub const SHOWN: usize = 5;
/// How many are kept, so a few going missing still leaves enough to show.
const KEPT: usize = 10;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentProjects {
    pub projects: Vec<PathBuf>,
}

impl RecentProjects {
    /// Next to `settings.toml`.
    pub fn user_path() -> Option<PathBuf> {
        Some(
            crate::settings::Settings::user_path()?
                .parent()?
                .join("recent.toml"),
        )
    }

    /// The saved list without folders that no longer exist; empty when
    /// there is no file yet.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let source =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut recent: Self =
            toml::from_str(&source).with_context(|| format!("parsing {}", path.display()))?;
        recent.projects.retain(|project| project.is_dir());
        Ok(recent)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let source = toml::to_string_pretty(self)?;
        std::fs::write(path, source).with_context(|| format!("writing {}", path.display()))
    }

    /// Move `project` to the front, adding it if it's new.
    pub fn push(&mut self, project: &Path) {
        self.remove(project);
        self.projects.insert(0, project.to_path_buf());
        self.projects.truncate(KEPT);
    }

    pub fn remove(&mut self, project: &Path) {
        self.projects.retain(|other| other != project);
    }
}

/// The live list and where it's saved.
struct AppRecent {
    recent: RecentProjects,
    /// `None` keeps changes in memory only, as in tests.
    path: Option<PathBuf>,
}

impl Global for AppRecent {}

/// Load the user's recent projects. An unreadable file starts an empty list
/// that isn't saved over it.
pub fn init(cx: &mut App) {
    let path = RecentProjects::user_path();
    let (recent, path) = match path.as_deref().map(RecentProjects::load) {
        Some(Ok(recent)) => (recent, path),
        Some(Err(error)) => {
            eprintln!("jig: recent projects weren't read: {error:#}");
            (RecentProjects::default(), None)
        }
        None => (RecentProjects::default(), None),
    };
    cx.set_global(AppRecent { recent, path });
}

/// The most recent projects, up to [`SHOWN`].
pub fn get(cx: &App) -> Vec<PathBuf> {
    cx.try_global::<AppRecent>()
        .map(|app| app.recent.projects.iter().take(SHOWN).cloned().collect())
        .unwrap_or_default()
}

/// Change the list and save it.
pub fn update(cx: &mut App, change: impl FnOnce(&mut RecentProjects)) {
    if !cx.has_global::<AppRecent>() {
        cx.set_global(AppRecent {
            recent: RecentProjects::default(),
            path: None,
        });
    }
    let app = cx.global_mut::<AppRecent>();
    let before = app.recent.clone();
    change(&mut app.recent);
    if app.recent == before {
        return;
    }
    if let Some(path) = &app.path
        && let Err(error) = app.recent.save(path)
    {
        eprintln!("jig: recent projects weren't saved: {error:#}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newest_first_without_duplicates() {
        let mut recent = RecentProjects::default();
        for name in ["a", "b", "a", "c"] {
            recent.push(Path::new(name));
        }
        assert_eq!(
            recent.projects,
            [Path::new("c"), Path::new("a"), Path::new("b")]
        );
        for n in 0..20 {
            recent.push(&PathBuf::from(n.to_string()));
        }
        assert_eq!(recent.projects.len(), KEPT);
        assert_eq!(recent.projects[0], Path::new("19"));
    }

    #[test]
    fn round_trips_and_drops_missing_folders() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jig").join("recent.toml");
        let mut recent = RecentProjects::default();
        recent.push(&dir.path().join("gone"));
        recent.push(dir.path());
        recent.save(&path).unwrap();
        assert_eq!(
            RecentProjects::load(&path).unwrap().projects,
            [dir.path().to_path_buf()]
        );
    }
}
