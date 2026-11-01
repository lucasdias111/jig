//! What's read from disk: a file's project root and `AGENTS.md`
//! ([`project`]), and the `.editorconfig` files above it ([`editorconfig`]).
//! Nothing here is kept; these are looked up each time.

pub mod editorconfig;
pub mod project;
