//! Text rules in plain Rust, tested without a window or a folder: matching
//! brackets, the preview diff, line edits, guessing indentation and what an
//! `.editorconfig` says.

pub mod brackets;
pub mod diff;
pub mod editorconfig;
pub mod indentation;
pub mod lines;
