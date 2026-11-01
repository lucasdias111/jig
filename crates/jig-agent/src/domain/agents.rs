//! The agents Jig can run, as data: `assets/default-agents.toml` has
//! the built-ins and documents every field, and the user's
//! `~/.config/jig/agents.toml` adds more or replaces one by `key`.
//! Settings > Agent picks one (`[agent] use`).

use std::sync::Arc;

use anyhow::{Result, bail};
use serde::Deserialize;

pub(crate) const BUILT_IN: &str = include_str!("../../../../assets/default-agents.toml");

/// The agent used when the settings name none, or one that's gone.
pub const DEFAULT: &str = jig_settings::DEFAULT_AGENT;

/// A starter for the user's `agents.toml`.
pub const USER_TEMPLATE: &str = r#"# Your agents for Jig. Any agent that speaks the Agent Client
# Protocol on stdio works; one with the same key as a built-in replaces it.
# See assets/default-agents.toml in Jig's source for the built-ins and
# every field. Pick one in Settings, under Agent.
#
# [[agent]]
# key = "goose"
# name = "Goose"
# command = ["goose", "acp"]
# about = "Block's Goose."
"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentFile {
    #[serde(default)]
    agent: Vec<Agent>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Jig's own OpenCode client, on a model from Jig's providers.
    OpenCode,
    /// The Agent Client Protocol on stdio.
    #[default]
    Acp,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    /// The program and its arguments.
    pub(crate) command: Vec<String>,
    #[serde(default)]
    pub(crate) env: std::collections::BTreeMap<String, String>,
    pub(crate) install: Option<String>,
    #[serde(default)]
    pub about: String,
}

impl Agent {
    pub fn installable(&self) -> bool {
        self.install.is_some()
    }

    /// Its install puts it in Jig's agents folder, rather than wherever its
    /// own installer does.
    pub fn installs_into_jig(&self) -> bool {
        self.install
            .as_deref()
            .is_some_and(|script| script.contains("${agents}"))
    }
}

pub struct Registry {
    pub agents: Vec<Arc<Agent>>,
    /// Why the user's file couldn't be read, if it couldn't.
    pub error: Option<String>,
}

impl Registry {
    pub fn get(&self, key: &str) -> Option<Arc<Agent>> {
        self.agents.iter().find(|agent| agent.key == key).cloned()
    }
}

/// The built-in agents, with those of the user's `agents.toml` (its text,
/// or why it couldn't be read) added or replacing them by key.
pub fn merged(user: Option<Result<String>>) -> Registry {
    let mut agents = parse(BUILT_IN).expect("the built-in agents parse");
    let mut error = None;
    match user.map(|text| text.and_then(|text| parse(&text))) {
        Some(Ok(mine)) => {
            for agent in mine {
                agents.retain(|other| other.key != agent.key);
                agents.push(agent);
            }
        }
        Some(Err(why)) => error = Some(format!("agents.toml: {why:#}")),
        None => {}
    }
    Registry {
        agents: agents.into_iter().map(Arc::new).collect(),
        error,
    }
}

fn parse(text: &str) -> Result<Vec<Agent>> {
    let file: AgentFile = toml::from_str(text)?;
    for agent in &file.agent {
        if agent.command.is_empty() {
            bail!("{} has no command", agent.name);
        }
    }
    Ok(file.agent)
}
