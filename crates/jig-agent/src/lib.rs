//! The coding agent in the app: agents as data, the AI providers, and the
//! shared OpenCode server.
//!
//! Laid out in `domain` (the agents and the pick, as data), `store` (the
//! files, the running agents) and `ui` (the GPUI globals); the modules
//! below keep the names the rest of Jig uses.

mod domain;
mod store;
mod ui;

/// The agent behind Jig's AI: OpenCode's server or an ACP agent's process,
/// started on first use, shared by every window, and stopped when Jig quits.
pub mod agent {
    pub use crate::domain::choice::{Choice, Pick};
    pub use crate::store::session::{connect, look_up_models, stop};
    pub use crate::store::terminal::open_in_terminal;
    pub use crate::ui::agent::{choice_for, init};
}

/// The agents Jig can run, as data: the built-ins and the user's
/// `agents.toml`.
pub mod agents {
    pub use crate::domain::agents::{Agent, DEFAULT, Kind, Registry, USER_TEMPLATE};
    pub use crate::store::persist::agents::{
        agents_dir, install, known_models, named, registry, remember_models, status, user_path,
    };
    pub use crate::ui::agent::chosen;
}

/// The AI providers, their saved keys and the models they offer.
pub mod providers {
    pub use crate::ui::providers::*;
}
