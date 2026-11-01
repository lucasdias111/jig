//! The debuggers Jig knows, as described in `assets/default-debuggers.toml`
//! and the user's `~/.config/jig/debuggers.toml`: how to start each one's
//! adapter, which run configurations it takes, how to install it, and the
//! `launch` request to send, with `${variables}` filled in. Any debugger
//! speaking the Debug Adapter Protocol can be added there; nothing about a
//! particular one lives in code but a few tools Jig finds for you.
//!
//! This file is the crate's facade. The work is in its layers:
//!
//! - [`domain`]: the debuggers as data, and filling in variables.
//! - [`store`]: the debuggers as they are now, installing them, and the
//!   tools they need.

mod domain;
mod store;

pub use domain::debuggers::{
    AdapterCommand, Debugger, Registry, USER_TEMPLATE, WITHOUT_STRUCT_SUMMARIES,
};
pub use domain::vars::{Tools, Vars};
pub use store::cache::registry;
pub use store::persist::{
    install, installed_files, lsp_bundles, missing, status, uninstall, user_path,
};
pub use store::programs::lldb_dap;

#[cfg(test)]
mod tests;
