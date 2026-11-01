//! `~/.config/jig/settings.toml`: the choices made in the Settings window.
//! Held in a global while Jig runs; every change is written back at once.
//! Beside it, `recent.toml` (the home page's projects) and `sessions.toml`
//! (each project's open files).
//!
//! This file is the crate's facade: it only says what the rest of Jig may
//! use. The work is in its layers (`docs/layers.md`):
//!
//! - [`ui`]: the app globals holding the live settings, and the shortcuts.
//! - [`domain`]: what the files hold, their defaults and the shortcuts'
//!   rules, in plain Rust.
//! - [`store`]: reading and writing the files.

mod domain;
mod store;
mod ui;

pub use domain::keyboard::Group as ShortcutGroup;
pub use domain::settings::*;
pub use ui::keyboard::{Shortcut, clashes as clashing_shortcuts, display as display_keys};
pub use ui::settings::{AppSettings, get, init, update};

/// `~/.config/jig/recent.toml`: the folders most recently opened as
/// projects, newest first, for the home page.
pub mod recent {
    pub use crate::domain::recent::{RecentProjects, SHOWN};
    pub use crate::ui::recent::{get, init, update};
}

/// `~/.config/jig/sessions.toml`: the files each project last had open, so
/// opening the folder again brings them back.
pub mod session {
    pub use crate::domain::session::{Session, Sessions};
    pub use crate::ui::session::{get, init, remember};
}
