//! The agents running in this process: OpenCode's server or an ACP agent's
//! process, started on first use, shared by every window, and stopped when
//! Jig quits.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use jig_ai::agent::acp::{AgentCommandLine, Connection};
use jig_ai::agent::harness::Backend;
use jig_ai::agent::opencode;
use serde_json::Value;

use crate::domain::choice::Choice;

/// The running OpenCode server and the provider config it was started with.
static SERVER: Mutex<Option<(Option<Value>, Arc<opencode::Server>)>> = Mutex::new(None);

/// Running ACP agents by key, with how each was started.
type Running = HashMap<String, (AgentCommandLine, Arc<Connection>)>;
static AGENTS: Mutex<Option<Running>> = Mutex::new(None);

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
        Choice::Acp {
            key, name, command, ..
        } => {
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

/// Ask an ACP agent which models it offers, by starting it and opening an
/// empty conversation, which agents don't keep until something is said in
/// it. Blocks, so call it off the main thread.
pub fn look_up_models(choice: &Choice) -> anyhow::Result<Option<jig_ai::agent::acp::AgentModels>> {
    let Choice::Acp { .. } = choice else {
        return Ok(None);
    };
    let backend = connect(choice)?;
    if let Backend::Acp(connection) = &backend {
        jig_ai::agent::acp::Session::create(connection.clone(), &std::env::temp_dir(), None)?;
    }
    Ok(backend.models())
}

/// The OpenCode server started with `config`, starting one if needed.
fn server(config: Option<&Value>) -> anyhow::Result<Arc<opencode::Server>> {
    let mut server = SERVER.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((started_with, server)) = server.as_ref()
        && started_with.as_ref() == config
    {
        return Ok(server.clone());
    }
    let started = Arc::new(opencode::Server::start(config)?);
    *server = Some((config.cloned(), started.clone()));
    Ok(started)
}
