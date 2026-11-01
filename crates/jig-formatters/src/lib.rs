//! The formatters Jig runs when a file's language server doesn't format:
//! `assets/default-formatters.toml` and the user's
//! `~/.config/jig/formatters.toml`, data like the debuggers. Each reads the
//! text on stdin and writes it formatted to stdout. Also the two tidy-ups
//! made on save, trailing whitespace and the final newline.
//!
//! This file is the crate's facade. The work is in its layers:
//!
//! - [`domain`]: the formatters as data, and tidying text.
//! - [`store`]: the formatters as they are now, and running one.

mod domain;
mod store;

pub use domain::formatters::{Formatter, Registry, USER_TEMPLATE};
pub use domain::tidy::tidy;
pub use store::cache::registry;
#[cfg(any(test, feature = "testing"))]
pub use store::cache::use_for_tests;
pub use store::persist::{formatters_dir, user_path};

#[cfg(test)]
mod tests;
