//! JSON-RPC with the agent process: requests and their answers, and the
//! messages it sends handed to the conversation they're for.

use std::io::Write as _;
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::{Connection, TIMEOUT, lock, read_file};
use crate::domain::acp::{AgentModels, commands};

pub(super) type Reply = Box<dyn FnOnce(Result<Value, RpcError>) + Send>;

/// What the agent sends a session.
pub(super) enum Incoming {
    Update(Value),
    /// An update from a subagent's session, handed to the conversation that
    /// started it.
    Subagent {
        session: String,
        update: Value,
    },
    /// A request to Jig, answered later with [`Connection::respond`].
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// The answer to the turn's `session/prompt`.
    Finished(Result<Value, RpcError>),
}

/// An error the agent answered with.
#[derive(Clone, Debug)]
pub(super) struct RpcError {
    pub(super) code: i64,
    pub(super) message: String,
}

impl RpcError {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            code: 0,
            message: message.into(),
        }
    }
}

impl Connection {
    pub(super) fn send(&self, message: &Value) -> Result<()> {
        if let Some(why) = lock(&self.gone).clone() {
            bail!("{why}");
        }
        let mut stdin = lock(&self.stdin);
        writeln!(stdin, "{message}")
            .and_then(|_| stdin.flush())
            .map_err(|_| anyhow!("{} stopped", self.name))
    }

    /// Send a request; `reply` gets its answer.
    pub(super) fn request_then(&self, method: &str, params: Value, reply: Reply) -> Result<()> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        lock(&self.pending).insert(id, reply);
        let sent =
            self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        if sent.is_err() {
            lock(&self.pending).remove(&id);
        }
        sent
    }

    /// Send a request and wait for its answer.
    pub(super) fn request(&self, method: &str, params: Value) -> Result<Value> {
        let (sender, receiver) = channel();
        self.request_then(
            method,
            params,
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        )?;
        receiver
            .recv_timeout(TIMEOUT)
            .map_err(|_| anyhow!("{} didn't answer", self.name))?
            .map_err(|error| self.failure(error))
    }

    pub(super) fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    pub(super) fn respond(&self, id: &Value, result: Result<Value, String>) -> Result<()> {
        self.send(&match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(message) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": message}})
            }
        })
    }

    /// One line from the agent.
    pub(super) fn receive(&self, line: &str) {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let method = message["method"].as_str();
        let mut session = message["params"]["sessionId"].as_str().map(str::to_string);
        // A subagent's messages go to the conversation that started it;
        // `subagent` keeps whose they are.
        let subagent = session
            .as_ref()
            .and_then(|session| lock(&self.subagents).get(session).cloned())
            .and_then(|conversation| session.replace(conversation));
        match (method, message.get("id")) {
            // An answer to Jig.
            (None, Some(id)) => {
                let Some(reply) = id.as_u64().and_then(|id| lock(&self.pending).remove(&id)) else {
                    return;
                };
                let error = &message["error"];
                reply(if error.is_null() {
                    Ok(message["result"].clone())
                } else {
                    Err(RpcError {
                        code: error["code"].as_i64().unwrap_or(0),
                        message: error["message"]
                            .as_str()
                            .unwrap_or("the agent failed")
                            .to_string(),
                    })
                });
            }
            (Some("session/update"), None) => {
                let update = &message["params"]["update"];
                if update["sessionUpdate"] == "config_option_update"
                    && let Some(models) = AgentModels::from_options(&update["configOptions"])
                {
                    *lock(&self.models) = Some(models);
                }
                if update["sessionUpdate"] == "available_commands_update" {
                    *lock(&self.commands) = commands(&update["availableCommands"]);
                }
                if update["sessionUpdate"] == "subagent_spawned"
                    && let (Some(child), Some(conversation)) =
                        (update["subagentSessionId"].as_str(), session.clone())
                {
                    lock(&self.subagents).insert(child.to_string(), conversation);
                }
                let incoming = match subagent {
                    Some(session) => Incoming::Subagent {
                        session,
                        update: update.clone(),
                    },
                    None => Incoming::Update(update.clone()),
                };
                self.route(session, incoming);
            }
            // Reading is safe to answer at once, from the disk.
            (Some("fs/read_text_file"), Some(id)) => {
                let _ = self.respond(id, read_file(&message["params"]));
            }
            (Some(method), Some(id)) => {
                let request = Incoming::Request {
                    id: id.clone(),
                    method: method.to_string(),
                    params: message["params"].clone(),
                };
                if !self.route(session, request) {
                    let _ = self.respond(id, Err("Jig isn't running that conversation.".into()));
                }
            }
            _ => {}
        }
    }

    /// Hand `incoming` to the session it's for. False when no one listens.
    pub(super) fn route(&self, session: Option<String>, incoming: Incoming) -> bool {
        let sessions = lock(&self.sessions);
        session
            .and_then(|session| sessions.get(&session))
            .is_some_and(|sender| sender.send(incoming).is_ok())
    }

    /// The process ended: everything waiting on it fails.
    pub(super) fn lost(&self) {
        let why = format!("{} stopped", self.name);
        *lock(&self.gone) = Some(why.clone());
        for (_, reply) in lock(&self.pending).drain() {
            reply(Err(RpcError::new(why.clone())));
        }
        for (_, sender) in lock(&self.sessions).drain() {
            let _ = sender.send(Incoming::Finished(Err(RpcError::new(why.clone()))));
        }
    }

    pub(super) fn listen(&self, session: &str) -> Receiver<Incoming> {
        let (sender, receiver) = channel();
        lock(&self.sessions).insert(session.to_string(), sender);
        receiver
    }
}
