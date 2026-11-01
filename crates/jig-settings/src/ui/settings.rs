//! The live settings, held in a global while Jig runs; every change is
//! written back at once.

use std::path::PathBuf;

use gpui_kit::{App, Global};

use crate::domain::settings::Settings;

/// The live settings and where they're saved.
pub struct AppSettings {
    settings: Settings,
    /// `None` keeps changes in memory only, as in tests.
    path: Option<PathBuf>,
}

impl Global for AppSettings {}

/// Load the user's settings into the app. Returns why they couldn't be read,
/// in which case the defaults are used and nothing is saved over the file.
pub fn init(cx: &mut App) -> Option<String> {
    let path = Settings::user_path();
    let (settings, path, error) = match path.as_deref().map(Settings::load) {
        Some(Ok(settings)) => (settings, path, None),
        Some(Err(error)) => (Settings::default(), None, Some(format!("{error:#}"))),
        None => (Settings::default(), None, None),
    };
    cx.set_global(AppSettings { settings, path });
    error
}

/// The current settings; the defaults until `init` has run.
pub fn get(cx: &App) -> Settings {
    cx.try_global::<AppSettings>()
        .map(|app| app.settings.clone())
        .unwrap_or_default()
}

/// Change the settings, save them, and notify everything observing them.
pub fn update(cx: &mut App, change: impl FnOnce(&mut Settings)) {
    if !cx.has_global::<AppSettings>() {
        cx.set_global(AppSettings {
            settings: Settings::default(),
            path: None,
        });
    }
    let app = cx.global_mut::<AppSettings>();
    let before = app.settings.clone();
    change(&mut app.settings);
    if app.settings == before {
        return;
    }
    if let Some(path) = &app.path
        && let Err(error) = app.settings.save(path)
    {
        eprintln!("jig: settings weren't saved: {error:#}");
    }
}
