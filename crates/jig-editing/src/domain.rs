//! Editing help as plain Rust: what to complete, snippets, what a server's
//! hover and signature help say, and edits as byte ranges. No GPUI and no
//! disk.

pub mod completions;
pub mod hover;
pub mod snippets;
pub mod text_edits;
