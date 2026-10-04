//! The OpenCode server behind agent commands: started on the first agent
//! command, shared by every window, and stopped when Jig quits.

use std::sync::{Arc, Mutex};

use gpui_kit::App;
use jig_ai::agent::AgentServer;
use serde_json::Value;

/// The running server and the provider config it was started with.
static SERVER: Mutex<Option<(Option<Value>, Arc<AgentServer>)>> = Mutex::new(None);

/// Stop the server on quit; a static is never dropped on its own.
pub fn init(cx: &mut App) {
    cx.on_app_quit(|_| {
        stop();
        async {}
    })
    .detach();
}

/// Stop the running server, if there is one.
pub fn stop() {
    if let Some((_, server)) = SERVER.lock().ok().and_then(|mut server| server.take()) {
        server.stop();
    }
}

/// A server started with `config`, starting one if needed. A server
/// started with other providers is replaced; runs still using it keep it
/// until they end. Blocks while it starts, so call it off the main thread.
pub fn server(config: Option<&Value>) -> anyhow::Result<Arc<AgentServer>> {
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
