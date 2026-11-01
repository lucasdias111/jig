//! One conversation with an ACP agent: starting, loading and running it,
//! and answering what the agent asks.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::channel;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde_json::{Value, json};

mod ask;

use super::rpc::Incoming;
use super::{Connection, lock};
use crate::domain::acp::{Transcript, TurnState, outcome, pick};
use crate::domain::agent::{AgentEvent, AgentRequest, Turn};
use crate::domain::map;
use crate::store::{canonical, session::command};

/// What a request the user is shown waits on.
enum Waiting {
    /// `edit` is the file, when it's an edit.
    Permission {
        id: Value,
        options: Value,
        edit: Option<PathBuf>,
    },
    Write {
        id: Value,
        path: PathBuf,
    },
}

/// One conversation with an ACP agent. Cheap to clone, so the UI can answer
/// requests and stop the agent while another thread runs [`Session::run`].
#[derive(Clone)]
pub struct Session {
    connection: Arc<Connection>,
    id: String,
    directory: String,
    waiting: Arc<Mutex<HashMap<String, Waiting>>>,
    /// Files whose edit the user allowed, which the agent may then hand Jig
    /// to write without asking again.
    allowed: Arc<Mutex<Vec<PathBuf>>>,
    next: Arc<AtomicU64>,
    /// Whether a turn or a load is under way. The connection hands a
    /// session's messages to one listener, so a second would take them
    /// from the first and leave both turns muddled.
    pub(super) busy: Arc<AtomicBool>,
}

/// A claim on a [`Session`] for one turn or load, given up when dropped.
struct Busy(Arc<AtomicBool>);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

impl Session {
    /// A new conversation, on `model` (one of [`Connection::models`]'
    /// values) when it's given and the agent offers it.
    pub fn create(
        connection: Arc<Connection>,
        directory: &Path,
        model: Option<&str>,
    ) -> Result<Self> {
        let directory = canonical(directory);
        let created = connection.request(
            "session/new",
            json!({"cwd": directory, "mcpServers": map::acp_servers(command().map(PathBuf::as_path), &directory)}),
        )?;
        connection.learn_models(&created);
        let id = created["sessionId"]
            .as_str()
            .context("The agent didn't return a session id")?
            .to_string();
        let session = Self::with(connection, id, directory);
        if let Some(model) = model {
            session.use_model(model)?;
        }
        Ok(session)
    }

    /// Switch the conversation to `model`, if the agent offers it and it
    /// isn't already on it.
    pub fn use_model(&self, model: &str) -> Result<()> {
        let Some(models) = self.connection.models() else {
            return Ok(());
        };
        if models.current == model || !models.options.iter().any(|(value, _)| value == model) {
            return Ok(());
        }
        let answer = self.connection.request(
            "session/set_config_option",
            json!({"sessionId": self.id, "configId": models.config_id, "value": model}),
        )?;
        self.connection.learn_models(&answer);
        Ok(())
    }

    /// An earlier conversation; [`Session::history`] picks it up.
    pub fn open(connection: Arc<Connection>, directory: &Path, id: &str) -> Self {
        Self::with(connection, id.to_string(), canonical(directory))
    }

    fn with(connection: Arc<Connection>, id: String, directory: String) -> Self {
        Self {
            connection,
            id,
            directory,
            waiting: Arc::default(),
            allowed: Arc::default(),
            next: Arc::default(),
            busy: Arc::default(),
        }
    }

    /// The session for one turn or load, unless one is already under way.
    fn claim(&self) -> Result<Busy> {
        if self.busy.swap(true, Ordering::SeqCst) {
            bail!(
                "{} is still working on this conversation.",
                self.connection.name
            );
        }
        Ok(Busy(self.busy.clone()))
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Load the conversation into the agent, and what was said in it, which
    /// the agent replays as it loads.
    pub fn history(&self) -> Result<Vec<Turn>> {
        if !self.connection.can("/loadSession") {
            bail!(
                "{} can't pick up earlier conversations.",
                self.connection.name
            );
        }
        let _busy = self.claim()?;
        let updates = self.connection.listen(&self.id);
        let (sender, done) = channel();
        self.connection.request_then(
            "session/load",
            json!({
                "sessionId": self.id,
                "cwd": self.directory,
                "mcpServers": map::acp_servers(command().map(PathBuf::as_path), &self.directory),
            }),
            Box::new(move |result| {
                let _ = sender.send(result);
            }),
        )?;
        let mut transcript = Transcript::new(&self.directory);
        loop {
            if let Ok(result) = done.try_recv() {
                // Updates sent before the answer are already queued.
                while let Ok(Incoming::Update(update)) = updates.try_recv() {
                    transcript.add(&update);
                }
                let loaded = result.map_err(|error| self.connection.failure(error))?;
                self.connection.learn_models(&loaded);
                return Ok(transcript.finish());
            }
            match updates.recv_timeout(Duration::from_millis(50)) {
                Ok(Incoming::Update(update)) => transcript.add(&update),
                Ok(Incoming::Request { id, .. }) => {
                    let _ = self
                        .connection
                        .respond(&id, Err("Not while loading.".into()));
                }
                Ok(Incoming::Finished(Err(error))) => return Err(self.connection.failure(error)),
                _ => {}
            }
        }
    }

    /// Send `request` and report what happens until the agent is done.
    /// Blocks; edits and other requests wait until [`Session::reply`].
    /// A second turn while one is under way is refused.
    pub fn run(&self, request: &AgentRequest, on_event: &dyn Fn(AgentEvent)) -> Result<()> {
        let _busy = self.claim()?;
        let incoming = self.connection.listen(&self.id);
        let text = match &request.command {
            Some(command) => format!("/{command} {}", request.prompt).trim().to_string(),
            None => request.prompt.clone(),
        };
        let finished = lock(&self.connection.sessions).get(&self.id).cloned();
        self.connection.request_then(
            "session/prompt",
            json!({"sessionId": self.id, "prompt": [{"type": "text", "text": text}]}),
            Box::new(move |result| {
                if let Some(sender) = finished {
                    let _ = sender.send(Incoming::Finished(result));
                }
            }),
        )?;
        let mut turn = TurnState::new(&self.directory, request.permissions);
        for message in incoming {
            match message {
                Incoming::Update(update) => {
                    for event in turn.update(&update) {
                        on_event(event);
                    }
                }
                Incoming::Subagent { session, update } => {
                    for event in turn.subagent_update(&session, &update) {
                        on_event(event);
                    }
                }
                Incoming::Request { id, method, params } => {
                    if let Some(event) = self.ask(&mut turn, id, &method, &params)? {
                        on_event(event);
                    }
                }
                Incoming::Finished(result) => {
                    let result = result.map_err(|error| self.connection.failure(error))?;
                    if result["stopReason"] == "refusal" {
                        bail!("{} refused.", self.connection.name);
                    }
                    on_event(AgentEvent::Done(turn.reply()));
                    return Ok(());
                }
            }
        }
        bail!("{} stopped", self.connection.name)
    }

    /// Let an edit or other request through, or turn it down. For an edit
    /// Jig writes itself, the file must already be written when accepting.
    pub fn reply(&self, request_id: &str, accept: bool, note: Option<&str>) -> Result<()> {
        let Some(waiting) = lock(&self.waiting).remove(request_id) else {
            return Ok(());
        };
        match waiting {
            Waiting::Permission { id, options, edit } => {
                if accept && let Some(path) = edit {
                    lock(&self.allowed).push(path);
                }
                let kinds: &[&str] = if accept {
                    &["allow_once", "allow_always"]
                } else {
                    &["reject_once", "reject_always"]
                };
                self.connection
                    .respond(&id, Ok(outcome(pick(&options, kinds).as_deref())))
            }
            Waiting::Write { id, path } => self.connection.respond(
                &id,
                if accept {
                    Ok(json!({}))
                } else {
                    Err(note.map(str::to_string).unwrap_or_else(|| {
                        format!("The user rejected the edit to {}.", path.display())
                    }))
                },
            ),
        }
    }

    /// Let a request through with the agent's own "don't ask again", or
    /// just this once when it has none.
    pub fn allow_always(&self, request_id: &str) -> Result<()> {
        let Some(Waiting::Permission { id, options, .. }) = lock(&self.waiting).remove(request_id)
        else {
            return Ok(());
        };
        let option = pick(&options, &["allow_always", "allow_once"]);
        self.connection.respond(&id, Ok(outcome(option.as_deref())))
    }

    /// Stop the agent's turn; whatever it was waiting on is called off.
    pub fn abort(&self) -> Result<()> {
        for (_, waiting) in lock(&self.waiting).drain() {
            let _ = match waiting {
                Waiting::Permission { id, .. } => self
                    .connection
                    .respond(&id, Ok(json!({"outcome": {"outcome": "cancelled"}}))),
                Waiting::Write { id, .. } => self
                    .connection
                    .respond(&id, Err("Stopped by the user.".into())),
            };
        }
        self.connection
            .notify("session/cancel", json!({"sessionId": self.id}))
    }
}
