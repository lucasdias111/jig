//! Starting other programs from a windowed app: without a console window of
//! their own on Windows, and, when they could hang, not forever.

mod no_window;
mod timed;

pub use no_window::NoWindow;
pub use timed::{describe, output_within};
