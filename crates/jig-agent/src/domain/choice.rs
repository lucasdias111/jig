//! Which agent a conversation runs on: picked in ⌘K or in Settings, and
//! how to start it.

use jig_ai::agent::acp::AgentCommandLine;
use jig_ai::agent::opencode;

use super::agents::DEFAULT;

/// An agent and model picked in ⌘K, in place of the settings', for the
/// window. `model` is `provider/model` for OpenCode, the agent's own name
/// for an ACP agent, or `None` for the agent's default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub agent: String,
    pub model: Option<String>,
}

/// Which agent to run and how, read on the main thread, where the settings
/// and keys are.
#[derive(Clone, Debug)]
pub enum Choice {
    OpenCode(opencode::Target),
    Acp {
        key: String,
        name: String,
        command: AgentCommandLine,
        /// Its model, by its own name; `None` for its default.
        model: Option<String>,
    },
}

impl Choice {
    /// The model to ask for, as OpenCode names it; ACP agents pick theirs
    /// as the conversation starts.
    pub fn model(&self) -> String {
        match self {
            Choice::OpenCode(target) => target.model.clone(),
            Choice::Acp { .. } => String::new(),
        }
    }

    /// An ACP agent's model, by its own name.
    pub fn acp_model(&self) -> Option<&str> {
        match self {
            Choice::OpenCode(_) => None,
            Choice::Acp { model, .. } => model.as_deref(),
        }
    }

    /// The agent's key, for what's remembered about it.
    pub fn key(&self) -> &str {
        match self {
            Choice::OpenCode(_) => DEFAULT,
            Choice::Acp { key, .. } => key,
        }
    }
}
