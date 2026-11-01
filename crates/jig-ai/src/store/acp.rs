//! The agent over the Agent Client Protocol: any agent that speaks ACP
//! (Claude Code and Codex through their adapters, Gemini CLI, OpenCode,
//! Goose, …) run as a child process and spoken to in JSON-RPC over stdio.
//!
//! Jig keeps the last word here too, as far as the agent lets it: an edit it
//! asks permission for, or hands Jig to write (`fs/write_text_file`), is
//! shown as a normal preview first. Other requests (shell commands, the
//! web) follow [`Permissions`]. An agent set to edit without asking can't
//! be stopped; Jig lists what it changed.

mod rpc;
mod session;

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::AtomicU64;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use jig_process::NoWindow as _;
use serde_json::{Value, json};

use super::canonical;
use crate::domain::acp::{self, AUTH_REQUIRED};
use crate::domain::agent::{AgentCommand, Conversation};
use rpc::{Incoming, Reply, RpcError};

pub use crate::domain::acp::{AgentCommandLine, AgentModels, SignIn, SignInNeeded};
pub use session::Session;

/// The protocol version Jig speaks.
const PROTOCOL_VERSION: u64 = 1;

/// How long a request other than a prompt may take.
const TIMEOUT: Duration = Duration::from_secs(120);

/// A running agent process. One serves every conversation with it; it is
/// stopped when dropped.
pub struct Connection {
    name: String,
    /// How it was started, which its terminal sign-in reuses.
    command: AgentCommandLine,
    /// How it says users sign in.
    sign_ins: Mutex<Vec<Value>>,
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, Reply>>,
    sessions: Mutex<HashMap<String, Sender<Incoming>>>,
    /// Each subagent's session, and the conversation's session it belongs
    /// to, however deep it was started.
    subagents: Mutex<HashMap<String, String>>,
    capabilities: Mutex<Value>,
    /// The commands it last said it has.
    commands: Mutex<Vec<AgentCommand>>,
    /// The models it last said it offers.
    models: Mutex<Option<AgentModels>>,
    /// Set once the process has gone.
    gone: Mutex<Option<String>>,
}

impl Connection {
    /// Start the agent and introduce Jig to it.
    pub fn start(name: &str, command: &AgentCommandLine) -> Result<Arc<Self>> {
        let mut process = Command::new(&command.program);
        process
            .no_window()
            .args(&command.args)
            .envs(command.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if let Some(path) = &command.path {
            process.env("PATH", path);
        }
        if let Some(home) = std::env::var_os("HOME") {
            process.current_dir(home);
        }
        let mut child = process
            .spawn()
            .with_context(|| format!("Couldn't start {name}. Is it installed?"))?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        let connection = Arc::new(Self {
            name: name.to_string(),
            command: command.clone(),
            sign_ins: Mutex::default(),
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            next_id: AtomicU64::new(0),
            pending: Mutex::default(),
            sessions: Mutex::default(),
            subagents: Mutex::default(),
            capabilities: Mutex::new(Value::Null),
            commands: Mutex::default(),
            models: Mutex::default(),
            gone: Mutex::default(),
        });
        let reader = Arc::downgrade(&connection);
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                let Some(connection) = reader.upgrade() else {
                    return;
                };
                connection.receive(&line);
            }
            if let Some(connection) = reader.upgrade() {
                connection.lost();
            }
        });
        let initialized = connection.request(
            "initialize",
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "clientCapabilities": {
                    "fs": {"readTextFile": true, "writeTextFile": true},
                    "terminal": false,
                    // Jig opens a terminal for an agent's own login.
                    "auth": {"terminal": true},
                    // Each subagent's work in a session of its own, shown
                    // apart in the conversation (the ACP draft's subagents).
                    "subagents": {},
                },
                "clientInfo": {"name": "jig", "title": "Jig", "version": env!("CARGO_PKG_VERSION")},
            }),
        )?;
        *lock(&connection.capabilities) = initialized["agentCapabilities"].clone();
        *lock(&connection.sign_ins) = initialized["authMethods"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        Ok(connection)
    }

    /// The ways to sign in the agent offers.
    pub fn sign_ins(&self) -> Vec<SignIn> {
        acp::sign_ins(&lock(&self.sign_ins), &self.command)
    }

    /// Ask the agent to sign in with `method`, or to pick up a sign-in
    /// just done in the terminal.
    pub fn authenticate(&self, method: &str) -> Result<()> {
        self.request("authenticate", json!({"methodId": method}))
            .map(|_| ())
    }

    /// The error to report for `error`: [`SignInNeeded`] when that's what it
    /// says.
    fn failure(&self, error: RpcError) -> anyhow::Error {
        if error.code == AUTH_REQUIRED && error.message.starts_with("Authentication required") {
            return anyhow::Error::new(SignInNeeded {
                agent: self.name.clone(),
                methods: self.sign_ins(),
            });
        }
        anyhow!("{}", error.message)
    }

    /// Still running.
    pub fn alive(&self) -> bool {
        lock(&self.gone).is_none()
    }

    pub fn stop(&self) {
        let mut child = lock(&self.child);
        let _ = child.kill();
        let _ = child.wait();
    }

    fn can(&self, pointer: &str) -> bool {
        let capabilities = lock(&self.capabilities);
        match capabilities.pointer(pointer) {
            Some(Value::Bool(on)) => *on,
            Some(Value::Null) | None => false,
            Some(_) => true,
        }
    }

    /// The agent's earlier conversations in `directory`, latest first, when
    /// it can list them.
    pub fn conversations(&self, directory: &Path) -> Result<Vec<Conversation>> {
        if !self.can("/sessionCapabilities/list") {
            return Ok(Vec::new());
        }
        let listed = self.request("session/list", json!({"cwd": canonical(directory)}))?;
        Ok(acp::conversations(&listed))
    }

    /// The commands and skills the agent last said it offers.
    pub fn commands(&self) -> Vec<AgentCommand> {
        lock(&self.commands).clone()
    }

    /// The models the agent last said it offers, if it lets clients pick.
    pub fn models(&self) -> Option<AgentModels> {
        lock(&self.models).clone()
    }

    fn learn_models(&self, response: &Value) {
        if let Some(models) = AgentModels::from_options(&response["configOptions"]) {
            *lock(&self.models) = Some(models);
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `fs/read_text_file`: the file, or the lines asked for.
fn read_file(params: &Value) -> Result<Value, String> {
    let path = params["path"].as_str().unwrap_or_default();
    let text = std::fs::read_to_string(path).map_err(|error| format!("{path}: {error}"))?;
    let line = params["line"].as_u64().unwrap_or(1).max(1) as usize;
    let content = match params["limit"].as_u64() {
        None if line == 1 => text,
        limit => text
            .split_inclusive('\n')
            .skip(line - 1)
            .take(limit.map_or(usize::MAX, |limit| limit as usize))
            .collect(),
    };
    Ok(json!({"content": content}))
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests;
