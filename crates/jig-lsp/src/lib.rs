//! Running language servers, as Neovim runs them, and guessing definitions
//! without one.
//!
//! This file is the crate's facade, with [`lsp`] and [`definitions`] the
//! names the rest of Jig uses. The work is in its layers:
//!
//! - [`ui`]: the running servers and their problems, as GPUI globals.
//! - [`domain`]: positions, URIs, the protocol's messages and guessing
//!   definitions, in plain Rust.
//! - [`store`]: each server's process and connection, its root and
//!   command, and installing it.

mod domain;
mod store;
mod ui;

pub mod definitions;
pub mod lsp;
