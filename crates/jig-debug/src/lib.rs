//! Debug sessions over the Debug Adapter Protocol: the client, picking a run
//! configuration's debugger, and debuggers living in a language server.
//!
//! This file is the crate's facade, with [`dap`], [`debug_target`] and
//! [`lsp_debug`] the names the rest of Jig uses. The work is in its layers:
//!
//! - [`domain`]: the protocol's messages, and planning a session.
//! - [`store`]: the adapters' connections and processes.

mod domain;
mod store;

/// Debug adapters, over their stdin and stdout or a TCP connection.
pub mod dap {
    pub use crate::domain::messages::{Message, frame, read_message};
    #[cfg(any(test, feature = "testing"))]
    pub use crate::store::client::socket_pair;
    pub use crate::store::client::{Client, Inbox, Server, free_port};

    #[cfg(test)]
    mod tests;
}

/// What debugging a run configuration means.
pub mod debug_target {
    pub use crate::domain::debug_target::{Build, DebugPlan, cargo_executable};
    pub use crate::store::programs::plan_for;
}

/// Debuggers that live inside a language server, as Java's does in jdtls.
pub mod lsp_debug {
    pub use crate::store::lsp_debug::{Started, start};
}
