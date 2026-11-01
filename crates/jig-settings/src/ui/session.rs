//! Each project's open files, held in a global while Jig runs and saved as
//! tabs change.

use std::path::{Path, PathBuf};

use gpui_kit::{App, Global};

use crate::domain::session::{Session, Sessions};

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
