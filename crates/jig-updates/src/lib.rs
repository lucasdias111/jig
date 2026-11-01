//! Updating Jig from its GitHub releases. Jig looks for a newer release
//! shortly after it starts and once a day (Settings > Updates turns that
//! off), downloads the build for this computer and checks it was signed
//! with Jig's key. Nothing is installed until you choose Restart to Update,
//! in the status bar or when Check for Updates… finds it.
//!
//! This file is the crate's facade: it only says what the rest of Jig may
//! use. The work is in its layers (`docs/layers.md`):
//!
//! - [`ui`]: the updates global, the checks it runs and its dialogs.
//! - [`domain`]: versions, signatures and where Jig is installed.
//! - [`store`]: GitHub, the download and installing it.

mod domain;
mod store;
mod ui;

pub use domain::release::Version;
#[cfg(any(test, feature = "testing"))]
pub use ui::updates::set_for_test;
pub use ui::updates::{
    CheckForUpdates, RestartToUpdate, State, Updates, check, current_version, init,
    open_releases_page, restart, state,
};
