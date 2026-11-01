//! The recent projects, held in a global while Jig runs, for the home
//! page.

use std::path::PathBuf;

use gpui_kit::{App, Global};

use crate::domain::recent::{RecentProjects, SHOWN};

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
