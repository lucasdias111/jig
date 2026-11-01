//! Run configurations: finding and running them, their output, and the
//! picker that chooses one.
//!
//! This file is the crate's facade, with [`run_configs`], [`run_output`]
//! and [`run_picker`] the names the rest of Jig uses. The work is in its
//! layers:
//!
//! - [`ui`]: the picker.
//! - [`domain`]: configurations and output lines, in plain Rust.
//! - [`store`]: detecting configurations, and running them.

mod domain;
mod store;
mod ui;

pub mod run_configs;
pub mod run_output;

/// The ⌃⌥R picker of run configurations.
pub mod run_picker {
    pub use crate::ui::run_picker::{PLAY, RunPicker, RunPickerEvent, WIDTH};
}
