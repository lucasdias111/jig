//! The OpenCode server behind agent commands: started on the first agent
//! command, shared by every window, and stopped when Jig quits.

use std::sync::{Arc, Mutex};

use gpui_kit::App;
use jig_ai::agent::AgentServer;

static SERVER: Mutex<Option<Arc<AgentServer>>> = Mutex::new(None);

/// Stop the server on quit; a static is never dropped on its own.
pub fn init(cx: &mut App) {
    cx.on_app_quit(|_| {
        if let Some(server) = SERVER.lock().ok().and_then(|mut server| server.take()) {
            server.stop();
        }
        async {}
    })
    .detach();
}

/// The running server, starting it if needed. Blocks while it starts, so
/// call it off the main thread.
pub fn server() -> anyhow::Result<Arc<AgentServer>> {
    let mut server = SERVER.lock().unwrap_or_else(|error| error.into_inner());
    if let Some(server) = server.as_ref() {
        return Ok(server.clone());
    }
    let started = Arc::new(AgentServer::start()?);
    *server = Some(started.clone());
    Ok(started)
}

/// The model agent commands use: `agent_model` in `config.toml`, or the
/// default.
pub fn model() -> String {
    jig_ai::Config::load(jig_ai::Config::user_path().as_deref())
        .ok()
        .and_then(|config| config.agent_model)
        .unwrap_or_else(|| jig_ai::agent::DEFAULT_MODEL.to_string())
}
