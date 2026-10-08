//! `~/.config/jig/sessions.toml`: the files each project last had open, in
//! tab order, and which one was showing, so opening the folder again brings
//! them back.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use gpui_kit::{App, Global};
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
        Some(
            crate::settings::Settings::user_path()?
                .parent()?
                .join("sessions.toml"),
        )
    }

    /// The saved sessions; none when there is no file yet.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let source =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&source).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let source = toml::to_string_pretty(self)?;
        std::fs::write(path, source).with_context(|| format!("writing {}", path.display()))
    }

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

/// The live sessions and where they're saved.
struct AppSessions {
    sessions: Sessions,
    /// `None` keeps changes in memory only, as in tests.
    path: Option<PathBuf>,
}

impl Global for AppSessions {}

/// Load the user's sessions. An unreadable file starts empty and isn't
/// saved over.
pub fn init(cx: &mut App) {
    let path = Sessions::user_path();
    let (sessions, path) = match path.as_deref().map(Sessions::load) {
        Some(Ok(sessions)) => (sessions, path),
        Some(Err(error)) => {
            eprintln!("jig: open files weren't read: {error:#}");
            (Sessions::default(), None)
        }
        None => (Sessions::default(), None),
    };
    cx.set_global(AppSessions { sessions, path });
}

/// What `root` last had open.
pub fn get(root: &Path, cx: &App) -> Option<Session> {
    cx.try_global::<AppSessions>()?.sessions.get(root).cloned()
}

/// Remember `session` for its project, saving only when it changed.
pub fn remember(session: Session, cx: &mut App) {
    if !cx.has_global::<AppSessions>() {
        cx.set_global(AppSessions {
            sessions: Sessions::default(),
            path: None,
        });
    }
    let app = cx.global_mut::<AppSessions>();
    if app.sessions.projects.first() == Some(&session) {
        return;
    }
    app.sessions.put(session);
    if let Some(path) = &app.path
        && let Err(error) = app.sessions.save(path)
    {
        eprintln!("jig: open files weren't saved: {error:#}");
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

    #[test]
    fn skips_missing_files_and_follows_the_active_one() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.rs", "c.rs"] {
            std::fs::write(dir.path().join(name), "").unwrap();
        }
        let session = Session {
            root: dir.path().to_path_buf(),
            files: ["a.rs", "gone.rs", "c.rs"].map(PathBuf::from).to_vec(),
            active: 2,
        };
        let (files, active) = session.existing_files();
        assert_eq!(files, [dir.path().join("a.rs"), dir.path().join("c.rs")]);
        assert_eq!(active, 1);
    }

    #[test]
    fn newest_first_one_per_project_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jig").join("sessions.toml");
        let mut sessions = Sessions::default();
        for (root, active) in [("a", 0), ("b", 0), ("a", 1)] {
            sessions.put(Session {
                root: root.into(),
                files: vec!["x.rs".into(), "y.rs".into()],
                active,
            });
        }
        assert_eq!(sessions.projects.len(), 2);
        assert_eq!(sessions.get(Path::new("a")).unwrap().active, 1);
        sessions.save(&path).unwrap();
        assert_eq!(Sessions::load(&path).unwrap(), sessions);
    }
}
