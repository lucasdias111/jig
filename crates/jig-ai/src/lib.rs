//! The AI layer: providers' config and keys, the coding agents, the task
//! planner's tasks, and how agents get the Map's drawing tools.
//!
//! Calls are blocking; the app runs them on a background thread. Laid out
//! in `domain` (what's decided, in plain Rust) and `store` (the agents'
//! processes, the files and the keychain); the modules below keep the names
//! the rest of Jig uses.

mod domain;
mod store;

/// Conversations with a coding agent, the way Jig's AI works.
pub mod agent {
    pub use crate::domain::agent::*;
    pub use crate::domain::read_only;
    pub use crate::store::harness;

    /// OpenCode, through Jig's own client: a private `opencode serve`
    /// spoken to over HTTP.
    pub mod opencode {
        pub use crate::domain::opencode::{DEFAULT_MODEL, Target};
        pub use crate::store::opencode::{Server, Session};
    }

    /// Any agent that speaks the Agent Client Protocol, over stdio.
    pub mod acp {
        pub use crate::store::acp::*;
    }
}

/// The task planner's tasks, one Markdown file each in `.jig/tasks/`.
pub mod tasks {
    pub use crate::domain::tasks::{FOLDER, Task, TaskStore, Todo};
    pub use crate::store::persist::tasks::{create, folder, list, load, path, save};

    /// The task planner as a tool for agents: an MCP server on stdio.
    pub mod mcp {
        pub use crate::domain::tasks::mcp::{CREATE, FLAG, NAME, TOOLS, project_from_args};
        pub use crate::store::persist::tasks::serve;
        pub use crate::store::session::{command, set_command};
    }
}

/// The Map's tools as agents get them.
pub mod map {
    pub use crate::domain::map::*;
}

/// `config.toml`: the providers Jig can reach, and the agent's model.
pub mod config {
    pub use crate::domain::config::*;
    pub use crate::store::session::session_id;
}

/// Editing `config.toml` from Settings, keeping its comments.
pub mod config_file {
    pub use crate::domain::config_file::*;
}

/// The keys typed in Settings, in the system keychain.
pub mod keys {
    pub use crate::store::persist::keys::*;
}

pub use config::{AgentModel, Config, ProviderConfig, ProviderKind, split_model};
pub use config_file::{ConfigFile, ProviderTemplate, TEMPLATES, template_named};
pub use keys::{KeySource, KeyStore};
