//! Agent mode over the Agent Client Protocol: any agent that speaks ACP
//! (Claude Code and Codex through their adapters, Gemini CLI, OpenCode,
//! Goose, …) run as a child process and spoken to in JSON-RPC over stdio.
//!
//! Jig keeps the last word here too, as far as the agent lets it: an edit it
//! asks permission for, or hands Jig to write (`fs/write_text_file`), is
//! shown as a normal preview first. Other requests (shell commands, the
//! web) follow [`Permissions`]. An agent set to edit without asking can't
//! be stopped; Jig lists what it changed.

use std::collections::HashMap;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::agent::{
    Access, AgentCommand, AgentEvent, AgentRequest, Conversation, EditChange, EditRequest,
    PermissionRequest, Permissions, Turn, canonical, relative,
};

/// The protocol version Jig speaks.
const PROTOCOL_VERSION: u64 = 1;

/// How long a request other than a prompt may take.
const TIMEOUT: Duration = Duration::from_secs(120);

/// How to start an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCommandLine {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// `PATH` for the agent, which may start other programs.
    pub path: Option<std::ffi::OsString>,
}

type Reply = Box<dyn FnOnce(Result<Value, String>) + Send>;

/// What the agent sends a session.
enum Incoming {
    Update(Value),
    /// A request to Jig, answered later with [`Connection::respond`].
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    /// The answer to the turn's `session/prompt`.
    Finished(Result<Value, String>),
}

/// A running agent process. One serves every conversation with it; it is
/// stopped when dropped.
pub struct Connection {
    name: String,
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    next_id: AtomicU64,
    pending: Mutex<HashMap<u64, Reply>>,
    sessions: Mutex<HashMap<String, Sender<Incoming>>>,
    capabilities: Mutex<Value>,
    /// The commands it last said it has.
    commands: Mutex<Vec<AgentCommand>>,
    /// Set once the process has gone.
    gone: Mutex<Option<String>>,
}

impl Connection {
    /// Start the agent and introduce Jig to it.
    pub fn start(name: &str, command: &AgentCommandLine) -> Result<Arc<Self>> {
        let mut process = Command::new(&command.program);
        process
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
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            next_id: AtomicU64::new(0),
            pending: Mutex::default(),
            sessions: Mutex::default(),
            capabilities: Mutex::new(Value::Null),
            commands: Mutex::default(),
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
                },
                "clientInfo": {"name": "jig", "title": "Jig", "version": env!("CARGO_PKG_VERSION")},
            }),
        )?;
        *lock(&connection.capabilities) = initialized["agentCapabilities"].clone();
        Ok(connection)
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
        let mut found: Vec<Conversation> = listed["sessions"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|session| {
                let title = session["title"].as_str().unwrap_or_default().trim();
                Some(Conversation {
                    id: session["sessionId"].as_str()?.to_string(),
                    // OpenCode's placeholder, "New session - <date>", says nothing.
                    title: (!title.is_empty() && !title.starts_with("New session"))
                        .then(|| title.to_string()),
                    updated: session["updatedAt"]
                        .as_str()
                        .and_then(parse_time)
                        .unwrap_or(SystemTime::UNIX_EPOCH),
                })
            })
            .collect();
        found.sort_by_key(|conversation| std::cmp::Reverse(conversation.updated));
        Ok(found)
    }

    /// The commands and skills the agent last said it offers.
    pub fn commands(&self) -> Vec<AgentCommand> {
        lock(&self.commands).clone()
    }

    fn send(&self, message: &Value) -> Result<()> {
        if let Some(why) = lock(&self.gone).clone() {
            bail!("{why}");
        }
        let mut stdin = lock(&self.stdin);
        writeln!(stdin, "{message}")
            .and_then(|_| stdin.flush())
            .map_err(|_| anyhow!("{} stopped", self.name))
    }

    /// Send a request; `reply` gets its answer.
    fn request_then(&self, method: &str, params: Value, reply: Reply) -> Result<()> {
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
    fn request(&self, method: &str, params: Value) -> Result<Value> {
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
            .map_err(|error| anyhow!("{error}"))
    }

    fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn respond(&self, id: &Value, result: Result<Value, String>) -> Result<()> {
        self.send(&match result {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err(message) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32000, "message": message}})
            }
        })
    }

    /// One line from the agent.
    fn receive(&self, line: &str) {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            return;
        };
        let method = message["method"].as_str();
        let session = message["params"]["sessionId"].as_str().map(str::to_string);
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
                    Err(error["message"]
                        .as_str()
                        .unwrap_or("the agent failed")
                        .to_string())
                });
            }
            (Some("session/update"), None) => {
                let update = &message["params"]["update"];
                if update["sessionUpdate"] == "available_commands_update" {
                    *lock(&self.commands) = commands(&update["availableCommands"]);
                }
                self.route(session, Incoming::Update(update.clone()));
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
    fn route(&self, session: Option<String>, incoming: Incoming) -> bool {
        let sessions = lock(&self.sessions);
        session
            .and_then(|session| sessions.get(&session))
            .is_some_and(|sender| sender.send(incoming).is_ok())
    }

    /// The process ended: everything waiting on it fails.
    fn lost(&self) {
        let why = format!("{} stopped", self.name);
        *lock(&self.gone) = Some(why.clone());
        for (_, reply) in lock(&self.pending).drain() {
            reply(Err(why.clone()));
        }
        for (_, sender) in lock(&self.sessions).drain() {
            let _ = sender.send(Incoming::Finished(Err(why.clone())));
        }
    }

    fn listen(&self, session: &str) -> Receiver<Incoming> {
        let (sender, receiver) = channel();
        lock(&self.sessions).insert(session.to_string(), sender);
        receiver
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.stop();
    }
}

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
}

impl Session {
    pub fn create(connection: Arc<Connection>, directory: &Path) -> Result<Self> {
        let directory = canonical(directory);
        let created =
            connection.request("session/new", json!({"cwd": directory, "mcpServers": []}))?;
        let id = created["sessionId"]
            .as_str()
            .context("The agent didn't return a session id")?
            .to_string();
        Ok(Self::with(connection, id, directory))
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
        }
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
        let updates = self.connection.listen(&self.id);
        let (sender, done) = channel();
        self.connection.request_then(
            "session/load",
            json!({"sessionId": self.id, "cwd": self.directory, "mcpServers": []}),
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
                result.map_err(|error| anyhow!("{error}"))?;
                return Ok(transcript.finish());
            }
            match updates.recv_timeout(Duration::from_millis(50)) {
                Ok(Incoming::Update(update)) => transcript.add(&update),
                Ok(Incoming::Request { id, .. }) => {
                    let _ = self
                        .connection
                        .respond(&id, Err("Not while loading.".into()));
                }
                Ok(Incoming::Finished(Err(error))) => bail!("{error}"),
                _ => {}
            }
        }
    }

    /// Send `request` and report what happens until the agent is done.
    /// Blocks; edits and other requests wait until [`Session::reply`].
    pub fn run(&self, request: &AgentRequest, on_event: &dyn Fn(AgentEvent)) -> Result<()> {
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
                Incoming::Request { id, method, params } => {
                    if let Some(event) = self.ask(&mut turn, id, &method, &params)? {
                        on_event(event);
                    }
                }
                Incoming::Finished(result) => {
                    let result = result.map_err(|error| anyhow!("{error}"))?;
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

    /// A request from the agent: answered at once when the settings say so,
    /// otherwise put to the user as an event.
    fn ask(
        &self,
        turn: &mut TurnState,
        id: Value,
        method: &str,
        params: &Value,
    ) -> Result<Option<AgentEvent>> {
        let ours = format!("acp-{}", self.next.fetch_add(1, Ordering::Relaxed));
        match method {
            "session/request_permission" => {
                let call = turn.merge(&params["toolCall"]);
                let options = params["options"].clone();
                let answer = |kinds: &[&str]| {
                    let option = pick(&options, kinds);
                    self.connection.respond(&id, Ok(outcome(option.as_deref())))
                };
                if let Some(edit) = call.edit() {
                    turn.reviewed(&call.id);
                    let waiting = Waiting::Permission {
                        id,
                        options,
                        edit: Some(edit.path.clone()),
                    };
                    lock(&self.waiting).insert(ours.clone(), waiting);
                    return Ok(Some(AgentEvent::Edit(EditRequest { id: ours, ..edit })));
                }
                match turn.permissions.access(call.kind.as_str()) {
                    Access::Allow => answer(&["allow_once", "allow_always"])?,
                    Access::Deny => answer(&["reject_once", "reject_always"])?,
                    Access::Ask => {
                        let waiting = Waiting::Permission {
                            id,
                            options,
                            edit: None,
                        };
                        lock(&self.waiting).insert(ours.clone(), waiting);
                        return Ok(Some(AgentEvent::Permission(PermissionRequest {
                            id: ours,
                            title: call.asking(),
                            detail: call.detail(&self.directory),
                        })));
                    }
                }
                Ok(None)
            }
            "fs/write_text_file" => {
                let path = PathBuf::from(params["path"].as_str().unwrap_or_default());
                let content = params["content"].as_str().unwrap_or_default().to_string();
                turn.wrote(&path);
                // An edit the user just allowed, which the agent leaves to Jig
                // to write: no need to ask twice.
                let allowed = {
                    let mut allowed = lock(&self.allowed);
                    let found = allowed.iter().position(|file| *file == path);
                    found.map(|at| allowed.remove(at)).is_some()
                };
                if allowed {
                    let written = std::fs::write(&path, &content)
                        .map(|()| json!({}))
                        .map_err(|error| format!("{}: {error}", path.display()));
                    self.connection.respond(&id, written)?;
                    return Ok(None);
                }
                lock(&self.waiting).insert(
                    ours.clone(),
                    Waiting::Write {
                        id,
                        path: path.clone(),
                    },
                );
                Ok(Some(AgentEvent::Edit(EditRequest {
                    id: ours,
                    path,
                    change: EditChange::Write(content),
                })))
            }
            _ => {
                self.connection
                    .respond(&id, Err("That isn't available inside Jig.".into()))?;
                Ok(None)
            }
        }
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

impl Permissions {
    /// What the settings say about a tool call of ACP kind `kind`.
    fn access(&self, kind: &str) -> Access {
        match kind {
            "execute" => self.shell,
            "fetch" => self.web_fetch,
            // Reading and searching the project needs no go-ahead.
            "read" | "search" | "think" => Access::Allow,
            _ => Access::Ask,
        }
    }
}

/// The option of the first of `kinds` the agent offers.
fn pick(options: &Value, kinds: &[&str]) -> Option<String> {
    kinds.iter().find_map(|kind| {
        options
            .as_array()?
            .iter()
            .find(|option| option["kind"] == *kind)?["optionId"]
            .as_str()
            .map(str::to_string)
    })
}

fn outcome(option: Option<&str>) -> Value {
    match option {
        Some(option) => json!({"outcome": {"outcome": "selected", "optionId": option}}),
        None => json!({"outcome": {"outcome": "cancelled"}}),
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

fn commands(list: &Value) -> Vec<AgentCommand> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|command| {
            Some(AgentCommand {
                name: command["name"]
                    .as_str()?
                    .trim_start_matches('/')
                    .to_string(),
                description: command["description"]
                    .as_str()
                    .unwrap_or_default()
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string(),
                skill: false,
            })
        })
        .collect()
}

/// A tool call as it has been described so far: `tool_call` and each
/// `tool_call_update` fill it in.
#[derive(Clone, Debug, Default)]
struct ToolCall {
    id: String,
    title: String,
    kind: String,
    status: String,
    raw_input: Value,
    content: Vec<Value>,
    locations: Vec<String>,
}

impl ToolCall {
    fn merge(&mut self, update: &Value) {
        let text = |name: &str| update[name].as_str().map(str::to_string);
        if let Some(id) = text("toolCallId") {
            self.id = id;
        }
        if let Some(title) = text("title") {
            self.title = title;
        }
        if let Some(kind) = text("kind") {
            self.kind = kind;
        }
        if let Some(status) = text("status") {
            self.status = status;
        }
        if update["rawInput"]
            .as_object()
            .is_some_and(|input| !input.is_empty())
        {
            self.raw_input = update["rawInput"].clone();
        }
        if let Some(content) = update["content"].as_array() {
            self.content = content.clone();
        }
        if let Some(locations) = update["locations"].as_array()
            && !locations.is_empty()
        {
            self.locations = locations
                .iter()
                .filter_map(|location| location["path"].as_str().map(str::to_string))
                .collect();
        }
    }

    /// The edit this call makes, when it says what it changes.
    fn edit(&self) -> Option<EditRequest> {
        if let Some(diff) = self
            .content
            .iter()
            .find(|content| content["type"] == "diff")
        {
            return Some(EditRequest {
                id: String::new(),
                path: PathBuf::from(diff["path"].as_str()?),
                change: EditChange::Replace {
                    old: diff["oldText"].as_str().map(str::to_string),
                    new: diff["newText"].as_str()?.to_string(),
                },
            });
        }
        // Agents that only describe the call, the way their own tools take it.
        let input = &self.raw_input;
        let path = input["file_path"]
            .as_str()
            .or_else(|| input["filePath"].as_str())
            .or_else(|| input["path"].as_str())?;
        let old = input["old_string"]
            .as_str()
            .or_else(|| input["oldString"].as_str());
        let new = input["new_string"]
            .as_str()
            .or_else(|| input["newString"].as_str())
            .or_else(|| input["content"].as_str())?;
        (self.kind == "edit" || old.is_some()).then(|| EditRequest {
            id: String::new(),
            path: PathBuf::from(path),
            change: EditChange::Replace {
                old: old.map(str::to_string),
                new: new.to_string(),
            },
        })
    }

    fn command(&self) -> Option<String> {
        let input = &self.raw_input;
        match &input["command"] {
            Value::String(command) => Some(command.clone()),
            Value::Array(words) => Some(
                words
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" "),
            ),
            _ => None,
        }
    }

    /// What it wants to do, as the user is asked it.
    fn asking(&self) -> String {
        match self.kind.as_str() {
            "execute" => "Run a command".into(),
            "fetch" => "Use the web".into(),
            "delete" => "Delete a file".into(),
            "move" => "Move a file".into(),
            "edit" => "Edit a file".into(),
            _ if self.title.is_empty() => "Use a tool".into(),
            _ => format!("Use {}", self.title),
        }
    }

    fn detail(&self, directory: &str) -> String {
        if let Some(command) = self.command() {
            return command;
        }
        if let Some(url) = self.raw_input["url"].as_str() {
            return url.into();
        }
        if let Some(path) = self.locations.first() {
            return relative(path, directory);
        }
        self.title.clone()
    }

    /// A short line for the UI while it runs.
    fn step(&self, directory: &str) -> String {
        let path = self.locations.first().map(|path| relative(path, directory));
        match (self.kind.as_str(), path) {
            ("read", Some(path)) => format!("Reading {path}"),
            ("edit", Some(path)) => format!("Editing {path}"),
            ("search", _) => "Searching".into(),
            ("execute", _) => "Running a command".into(),
            ("fetch", _) => "Looking on the web".into(),
            ("think", _) => "Thinking".into(),
            _ if self.title.is_empty() => "Working".into(),
            _ => self.title.clone(),
        }
    }

    /// The transcript's line once it's done, for what's worth keeping.
    fn done(&self, directory: &str) -> Option<String> {
        match self.kind.as_str() {
            "execute" => Some(format!(
                "Ran `{}`",
                self.command().unwrap_or_else(|| self.title.clone())
            )),
            "fetch" => Some(format!("Fetched {}", self.detail(directory))),
            "delete" => Some(format!("Deleted {}", self.detail(directory))),
            "move" => Some(format!("Moved {}", self.detail(directory))),
            _ => None,
        }
    }

    fn edited_path(&self, directory: &str) -> Option<String> {
        self.content
            .iter()
            .find(|content| content["type"] == "diff")
            .and_then(|diff| diff["path"].as_str())
            .or_else(|| self.locations.first().map(String::as_str))
            .map(|path| relative(path, directory))
    }
}

/// What a turn has seen so far.
struct TurnState {
    directory: String,
    permissions: Permissions,
    calls: HashMap<String, ToolCall>,
    /// Tool calls and files the user reviewed, so they aren't listed as
    /// changed without asking.
    reviewed: Vec<String>,
    written: Vec<PathBuf>,
    /// The agent's words, message by message.
    messages: Vec<(Option<String>, String)>,
}

impl TurnState {
    fn new(directory: &str, permissions: Permissions) -> Self {
        Self {
            directory: directory.to_string(),
            permissions,
            calls: HashMap::new(),
            reviewed: Vec::new(),
            written: Vec::new(),
            messages: Vec::new(),
        }
    }

    fn merge(&mut self, update: &Value) -> ToolCall {
        let id = update["toolCallId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let call = self.calls.entry(id).or_default();
        call.merge(update);
        call.clone()
    }

    fn reviewed(&mut self, call: &str) {
        self.reviewed.push(call.to_string());
    }

    fn wrote(&mut self, path: &Path) {
        self.written.push(path.to_path_buf());
    }

    fn update(&mut self, update: &Value) -> Vec<AgentEvent> {
        match update["sessionUpdate"].as_str().unwrap_or_default() {
            "agent_message_chunk" => {
                add_chunk(&mut self.messages, update);
                Vec::new()
            }
            "agent_thought_chunk" => vec![AgentEvent::Step("Thinking".into())],
            "plan" => vec![AgentEvent::Step("Planning".into())],
            "available_commands_update" => {
                vec![AgentEvent::Commands(commands(&update["availableCommands"]))]
            }
            "tool_call" | "tool_call_update" => {
                let call = self.merge(update);
                match call.status.as_str() {
                    "completed" if call.kind == "edit" => {
                        let asked = self.reviewed.contains(&call.id)
                            || call.content.iter().any(|content| {
                                content["path"].as_str().is_some_and(|path| {
                                    self.written.iter().any(|w| w == Path::new(path))
                                })
                            });
                        match call.edited_path(&self.directory) {
                            Some(path) if !asked => {
                                vec![AgentEvent::Did(format!("Edited {path} without asking"))]
                            }
                            _ => Vec::new(),
                        }
                    }
                    "completed" => call
                        .done(&self.directory)
                        .map(AgentEvent::Did)
                        .into_iter()
                        .collect(),
                    "failed" => Vec::new(),
                    _ => vec![AgentEvent::Step(call.step(&self.directory))],
                }
            }
            _ => Vec::new(),
        }
    }

    fn reply(&self) -> String {
        join(&self.messages)
    }
}

/// Add a message chunk to `messages`, starting a new message when its id
/// changes.
fn add_chunk(messages: &mut Vec<(Option<String>, String)>, update: &Value) {
    let text = update["content"]["text"].as_str().unwrap_or_default();
    let id = update["messageId"].as_str().map(str::to_string);
    match messages.last_mut() {
        Some((last, words)) if *last == id => words.push_str(text),
        _ => messages.push((id, text.to_string())),
    }
}

fn join(messages: &[(Option<String>, String)]) -> String {
    messages
        .iter()
        .map(|(_, text)| text.trim())
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// An earlier conversation as the agent replays it.
struct Transcript {
    directory: String,
    turns: Vec<crate::agent::Turn>,
    calls: HashMap<String, ToolCall>,
    /// The message being replayed, by who said it and its id.
    current: Option<(bool, Option<String>)>,
}

impl Transcript {
    fn new(directory: &str) -> Self {
        Self {
            directory: directory.to_string(),
            turns: Vec::new(),
            calls: HashMap::new(),
            current: None,
        }
    }

    fn add(&mut self, update: &Value) {
        use crate::agent::Turn as Line;
        let kind = update["sessionUpdate"].as_str().unwrap_or_default();
        let user = match kind {
            "user_message_chunk" => true,
            "agent_message_chunk" => false,
            "tool_call" | "tool_call_update" => {
                self.current = None;
                let id = update["toolCallId"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let call = self.calls.entry(id).or_default();
                call.merge(update);
                let call = call.clone();
                let line = match call.status.as_str() {
                    "completed" | "failed" if call.kind == "edit" => {
                        call.edited_path(&self.directory).map(|path| Line::Edit {
                            path,
                            accepted: call.status == "completed",
                        })
                    }
                    "completed" => call.done(&self.directory).map(Line::Did),
                    _ => None,
                };
                self.turns.extend(line);
                return;
            }
            _ => return,
        };
        let text = update["content"]["text"].as_str().unwrap_or_default();
        let id = update["messageId"].as_str().map(str::to_string);
        let same = self.current.as_ref() == Some(&(user, id.clone()));
        match self.turns.last_mut() {
            Some(Line::User(words)) if same && user => words.push_str(text),
            Some(Line::Agent(words)) if same && !user => words.push_str(text),
            _ => self.turns.push(if user {
                Line::User(text.to_string())
            } else {
                Line::Agent(text.to_string())
            }),
        }
        self.current = Some((user, id));
    }

    /// The lines, without what Jig adds about where the user is, and
    /// without the agent's words split by whitespace.
    fn finish(self) -> Vec<crate::agent::Turn> {
        use crate::agent::Turn as Line;
        self.turns
            .into_iter()
            .filter_map(|line| match line {
                Line::User(text) => Some(Line::User(crate::agent::asked(&text).to_string())),
                Line::Agent(text) if text.trim().is_empty() => None,
                Line::Agent(text) => Some(Line::Agent(text.trim().to_string())),
                other => Some(other),
            })
            .collect()
    }
}

/// An ISO 8601 time such as "2026-10-10T05:38:50.337Z", in UTC.
fn parse_time(text: &str) -> Option<SystemTime> {
    let (date, time) = text.split_once('T')?;
    let mut date = date.split('-').map(|part| part.parse::<i64>());
    let (year, month, day) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let time = time.trim_end_matches('Z');
    let time = time.split(['+']).next()?;
    let mut parts = time.split(':');
    let hour: i64 = parts.next()?.parse().ok()?;
    let minute: i64 = parts.next()?.parse().ok()?;
    let second: f64 = parts.next().unwrap_or("0").parse().ok()?;
    // Days since 1970-01-01, from Howard Hinnant's civil calendar.
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let of_era = year - era * 400;
    let of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let of_cycle = of_era * 365 + of_era / 4 - of_era / 100 + of_year;
    let days = era * 146_097 + of_cycle - 719_468;
    let seconds = days * 86_400 + hour * 3600 + minute * 60;
    let at = Duration::from_secs(u64::try_from(seconds).ok()?) + Duration::from_secs_f64(second);
    Some(SystemTime::UNIX_EPOCH + at)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iso_times() {
        let at = parse_time("2026-10-10T05:38:50.337Z").unwrap();
        let seconds = at.duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        assert_eq!(seconds, 1_791_610_730);
        assert_eq!(
            parse_time("1970-01-02T00:00:00Z"),
            Some(SystemTime::UNIX_EPOCH + Duration::from_secs(86_400))
        );
        assert_eq!(parse_time("yesterday"), None);
    }

    #[test]
    fn an_edit_comes_from_its_diff_or_its_input() {
        let mut call = ToolCall::default();
        call.merge(&json!({
            "toolCallId": "c1", "kind": "edit", "title": "edit",
            "content": [{"type": "diff", "path": "/p/a.rs", "oldText": "a", "newText": "b"}],
        }));
        let edit = call.edit().unwrap();
        assert_eq!(edit.path, PathBuf::from("/p/a.rs"));
        assert_eq!(
            edit.change,
            EditChange::Replace {
                old: Some("a".into()),
                new: "b".into()
            }
        );

        let mut call = ToolCall::default();
        call.merge(&json!({
            "toolCallId": "c2", "kind": "edit",
            "rawInput": {"file_path": "/p/b.rs", "old_string": "x", "new_string": "y"},
        }));
        assert_eq!(call.edit().unwrap().path, PathBuf::from("/p/b.rs"));

        let mut run = ToolCall::default();
        run.merge(&json!({"toolCallId": "c3", "kind": "execute", "rawInput": {"command": ["cargo", "test"]}}));
        assert!(run.edit().is_none());
        assert_eq!(run.detail("/p"), "cargo test");
        assert_eq!(run.asking(), "Run a command");
    }

    #[test]
    fn a_turn_collects_the_reply_and_flags_edits_made_without_asking() {
        let mut turn = TurnState::new("/p", Permissions::default());
        let chunk = |id: &str, text: &str| json!({"sessionUpdate": "agent_message_chunk", "messageId": id, "content": {"type": "text", "text": text}});
        turn.update(&chunk("m1", "Looking"));
        turn.update(&chunk("m1", " now."));
        turn.update(&chunk("m2", "Done."));
        assert_eq!(turn.reply(), "Looking now.\n\nDone.");

        let events = turn.update(&json!({
            "sessionUpdate": "tool_call_update", "toolCallId": "e1", "kind": "edit", "status": "completed",
            "content": [{"type": "diff", "path": "/p/src/a.rs", "oldText": "a", "newText": "b"}],
        }));
        assert_eq!(
            events,
            [AgentEvent::Did("Edited src/a.rs without asking".into())]
        );

        turn.reviewed("e2");
        let events = turn.update(&json!({
            "sessionUpdate": "tool_call_update", "toolCallId": "e2", "kind": "edit", "status": "completed",
            "content": [{"type": "diff", "path": "/p/src/b.rs", "newText": "b"}],
        }));
        assert!(events.is_empty(), "it was reviewed");
    }

    #[test]
    fn settings_decide_by_the_kind_of_tool() {
        let permissions = Permissions {
            shell: Access::Deny,
            web_fetch: Access::Allow,
            ..Permissions::default()
        };
        assert_eq!(permissions.access("execute"), Access::Deny);
        assert_eq!(permissions.access("fetch"), Access::Allow);
        assert_eq!(permissions.access("read"), Access::Allow);
        assert_eq!(permissions.access("delete"), Access::Ask);
    }

    #[test]
    fn a_replayed_conversation_becomes_the_transcript() {
        let mut transcript = Transcript::new("/p");
        for update in [
            json!({"sessionUpdate": "user_message_chunk", "content": {"type": "text", "text": "Add docs\n\nI'm in a.rs, with line 1 selected"}}),
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "On it"}}),
            json!({"sessionUpdate": "tool_call", "toolCallId": "t", "kind": "edit", "status": "completed",
                   "content": [{"type": "diff", "path": "/p/a.rs", "newText": "x"}]}),
            json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Done."}}),
        ] {
            transcript.add(&update);
        }
        use crate::agent::Turn as Line;
        assert_eq!(
            transcript.finish(),
            [
                Line::User("Add docs".into()),
                Line::Agent("On it".into()),
                Line::Edit {
                    path: "a.rs".into(),
                    accepted: true
                },
                Line::Agent("Done.".into()),
            ]
        );
    }

    #[test]
    fn reads_the_lines_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f.txt");
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        let read = |line: Option<u64>, limit: Option<u64>| {
            read_file(&json!({"path": path, "line": line, "limit": limit})).unwrap()["content"]
                .as_str()
                .unwrap()
                .to_string()
        };
        assert_eq!(read(None, None), "one\ntwo\nthree\n");
        assert_eq!(read(Some(2), Some(1)), "two\n");
    }
}
