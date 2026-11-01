//! The Settings and About windows.
//!
//! This file is the crate's facade: it only says what the rest of Jig may
//! use. The work is in its layers (`docs/layers.md`):
//!
//! - [`ui`]: the two windows and every Settings page.
//! - [`domain`]: the little wording the pages share, in plain Rust.
//! - [`store`]: the snapshots' scratch folders (with the `snapshots` feature).

mod domain;
#[cfg(feature = "snapshots")]
mod store;
mod ui;

pub use ui::{about_window, settings_window};
