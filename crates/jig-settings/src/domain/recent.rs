//! `~/.config/jig/recent.toml`: the folders most recently opened as
//! projects, newest first, for the home page.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many the home page lists.
pub const SHOWN: usize = 5;
/// How many are kept, so a few going missing still leaves enough to show.
pub(crate) const KEPT: usize = 10;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecentProjects {
    pub projects: Vec<PathBuf>,
}

impl RecentProjects {
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
}
