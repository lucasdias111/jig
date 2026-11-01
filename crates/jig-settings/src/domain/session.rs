//! `~/.config/jig/sessions.toml`: the files each project last had open, in
//! tab order, and which one was showing, so opening the folder again brings
//! them back.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many projects are remembered, most recently changed first.
const KEPT: usize = 20;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sessions {
    #[serde(rename = "project")]
    pub projects: Vec<Session>,
}

/// One project's open files.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub root: PathBuf,
    /// Relative to `root` when inside it, so a moved project keeps its tabs.
    pub files: Vec<PathBuf>,
    /// Index into `files` of the tab that was showing.
    pub active: usize,
}

impl Session {
    /// A session for `root` with `files` (full paths) open.
    pub fn new(root: &Path, files: &[PathBuf], active: usize) -> Self {
        let files = files
            .iter()
            .map(|file| {
                file.strip_prefix(root)
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|_| file.clone())
            })
            .collect();
        Self {
            root: root.to_path_buf(),
            files,
            active,
        }
    }
}

impl Sessions {
    pub fn get(&self, root: &Path) -> Option<&Session> {
        self.projects.iter().find(|session| session.root == root)
    }

    /// Replace `session`'s project's entry and move it to the front.
    pub fn put(&mut self, session: Session) {
        self.projects.retain(|other| other.root != session.root);
        self.projects.insert(0, session);
        self.projects.truncate(KEPT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_files_relative_to_the_project() {
        let session = Session::new(
            Path::new("/p"),
            &[
                PathBuf::from("/p/src/a.rs"),
                PathBuf::from("/elsewhere/b.rs"),
            ],
            1,
        );
        assert_eq!(
            session.files,
            [Path::new("src/a.rs"), Path::new("/elsewhere/b.rs")]
        );
    }
}
