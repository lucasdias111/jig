//! The agent behind Agent mode: OpenCode's server or an ACP agent's
//! process, started on first use, shared by every window, and stopped when
//! Jig quits.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui_kit::App;
use jig_ai::acp::{AgentCommandLine, Connection};
use jig_ai::agent::{AgentServer, AgentTarget};
use jig_ai::harness::Backend;
use serde_json::Value;

use crate::agents::Kind;

/// The running OpenCode server and the provider config it was started with.
static SERVER: Mutex<Option<(Option<Value>, Arc<AgentServer>)>> = Mutex::new(None);

/// Running ACP agents by key, with how each was started.
type Running = HashMap<String, (AgentCommandLine, Arc<Connection>)>;
static AGENTS: Mutex<Option<Running>> = Mutex::new(None);

/// Which agent to run and how, read on the main thread, where the settings
/// and keys are.
#[derive(Clone, Debug)]
pub enum Choice {
    OpenCode(AgentTarget),
    Acp {
        key: String,
        name: String,
        command: AgentCommandLine,
    },
}

impl Choice {
    /// The model to ask for, as OpenCode names it; ACP agents use their own.
    pub fn model(&self) -> String {
        match self {
            Choice::OpenCode(target) => target.model.clone(),
            Choice::Acp { .. } => String::new(),
        }
    }
}

/// The agent Settings picked, ready to start, or why it can't.
pub fn choice(cx: &App) -> Result<Choice, String> {
    let agent = crate::agents::chosen(cx);
    match agent.kind {
        Kind::OpenCode => crate::providers::agent_target(cx).map(Choice::OpenCode),
        Kind::Acp => Ok(Choice::Acp {
            key: agent.key.clone(),
            name: agent.name.clone(),
            command: agent
                .command_line()
                .map_err(|missing| format!("{missing} Install it in Settings > Agent."))?,
        }),
    }
}

/// Stop the servers on quit; a static is never dropped on its own.
pub fn init(cx: &mut App) {
    cx.on_app_quit(|_| {
        stop();
        async {}
    })
    .detach();
}

/// Stop every running agent.
pub fn stop() {
    if let Some((_, server)) = SERVER.lock().ok().and_then(|mut server| server.take()) {
        server.stop();
    }
    if let Some(agents) = AGENTS.lock().ok().and_then(|mut agents| agents.take()) {
        for (_, connection) in agents.values() {
            connection.stop();
        }
    }
}

/// The agent for `choice`, starting it if needed. One started another way
/// (other providers, another command) is replaced; runs still using it keep
/// it until they end. Blocks while it starts, so call it off the main thread.
pub fn connect(choice: &Choice) -> anyhow::Result<Backend> {
    match choice {
        Choice::OpenCode(target) => server(target.config.as_ref()).map(Backend::OpenCode),
        Choice::Acp { key, name, command } => {
            let mut agents = AGENTS.lock().unwrap_or_else(|error| error.into_inner());
            let agents = agents.get_or_insert_with(HashMap::new);
            if let Some((started_with, connection)) = agents.get(key)
                && started_with == command
                && connection.alive()
            {
                return Ok(Backend::Acp(connection.clone()));
            }
            let connection = Connection::start(name, command)?;
            agents.insert(key.clone(), (command.clone(), connection.clone()));
            Ok(Backend::Acp(connection))
        }
    }
}

/// The OpenCode server started with `config`, starting one if needed.
fn server(config: Option<&Value>) -> anyhow::Result<Arc<AgentServer>> {
    let mut server = SERVER.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((started_with, server)) = server.as_ref()
        && started_with.as_ref() == config
    {
        return Ok(server.clone());
    }
    let started = Arc::new(AgentServer::start(config)?);
    *server = Some((config.cloned(), started.clone()));
    Ok(started)
}
