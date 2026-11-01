//! Starting other programs from a windowed app: without a console window of
//! their own on Windows, and, when they could hang, not forever.
//!
//! This file is the crate's facade. All the work is process IO, so it's
//! all in [`store`]; there's no `ui` or `domain` (`docs/layers.md`).

mod store;

pub use store::no_window::NoWindow;
pub use store::timed::{describe, output_within};
