//! The agent over the Agent Client Protocol: any agent that speaks ACP
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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, anyhow, bail};
use jig_process::NoWindow as _;
use serde_json::{Value, json};

use super::{
    Access, AgentCommand, AgentEvent, AgentRequest, Asking, Conversation, EditChange, EditRequest,
    PermissionRequest, Permissions, Turn, canonical, relative,
};
use crate::{map, tasks};

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

/// An error the agent answered with.
#[derive(Clone, Debug)]
struct RpcError {
    code: i64,
    message: String,
}

impl RpcError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            code: 0,
            message: message.into(),
        }
    }
}

/// ACP's code for "sign in first".
const AUTH_REQUIRED: i64 = -32000;

/// The agent needs the user to sign in before it can work. Carried in the
/// `anyhow::Error` of whatever asked, for the UI to offer [`SignIn`]s.
#[derive(Clone, Debug)]
pub struct SignInNeeded {
    pub agent: String,
    pub methods: Vec<SignIn>,
}

impl std::fmt::Display for SignInNeeded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} needs you to sign in.", self.agent)
    }
}

impl std::error::Error for SignInNeeded {}

/// One way to sign in to an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SignIn {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Run this in a terminal, where the agent's own login takes over; the
    /// agent then works once it's done. `None`: the agent signs in itself
    /// when asked ([`Connection::authenticate`]), e.g. in the browser.
    pub terminal: Option<AgentCommandLine>,
}

type Reply = Box<dyn FnOnce(Result<Value, RpcError>) + Send>;

/// What the agent sends a session.
enum Incoming {
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
        lock(&self.sign_ins)
            .iter()
            .filter_map(|method| {
                let terminal = (method["type"] == "terminal").then(|| AgentCommandLine {
                    args: self
                        .command
                        .args
                        .iter()
                        .cloned()
                        .chain(
                            method["args"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(|arg| arg.as_str().map(str::to_string)),
                        )
                        .collect(),
                    ..self.command.clone()
                });
                Some(SignIn {
                    id: method["id"].as_str()?.to_string(),
                    name: method["name"]
                        .as_str()
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                    description: method["description"]
                        .as_str()
                        .unwrap_or_default()
                        .trim()
                        .to_string(),
                    terminal,
                })
            })
            .collect()
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

    /// The models the agent last said it offers, if it lets clients pick.
    pub fn models(&self) -> Option<AgentModels> {
        lock(&self.models).clone()
    }

    fn learn_models(&self, response: &Value) {
        if let Some(models) = AgentModels::from_options(&response["configOptions"]) {
            *lock(&self.models) = Some(models);
        }
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
            .map_err(|error| self.failure(error))
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
            reply(Err(RpcError::new(why.clone())));
        }
        for (_, sender) in lock(&self.sessions).drain() {
            let _ = sender.send(Incoming::Finished(Err(RpcError::new(why.clone()))));
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

/// The models an agent offers, as the setting it lets clients change.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentModels {
    /// The setting's id, for `session/set_config_option`.
    pub config_id: String,
    /// The value it's on.
    pub current: String,
    /// Each model's value and name, e.g. ("sonnet", "Sonnet 5.5").
    pub options: Vec<(String, String)>,
}

impl AgentModels {
    /// The model setting among an agent's `configOptions`, if it has one.
    fn from_options(options: &Value) -> Option<Self> {
        let setting = options
            .as_array()?
            .iter()
            .find(|option| option["category"] == "model" && option["type"] == "select")?;
        let mut choices = Vec::new();
        for option in setting["options"].as_array().into_iter().flatten() {
            // Options may come in groups.
            let group = option["options"].as_array();
            for choice in group
                .into_iter()
                .flatten()
                .chain(group.is_none().then_some(option))
            {
                if let Some(value) = choice["value"].as_str() {
                    let name = choice["name"].as_str().unwrap_or(value);
                    choices.push((value.to_string(), name.to_string()));
                }
            }
        }
        Some(Self {
            config_id: setting["id"].as_str()?.to_string(),
            current: setting["currentValue"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            options: choices,
        })
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
    /// Whether a turn or a load is under way. The connection hands a
    /// session's messages to one listener, so a second would take them
    /// from the first and leave both turns muddled.
    busy: Arc<AtomicBool>,
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
            json!({"cwd": directory, "mcpServers": map::acp_servers(&directory)}),
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
                "mcpServers": map::acp_servers(&self.directory),
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
                let access = if call.map_tool().is_some() {
                    // Drawing on the Map changes no code.
                    Access::Allow
                } else if call.is_task_tool() {
                    turn.permissions.tasks
                } else {
                    turn.permissions.access(call.kind.as_str())
                };
                match access {
                    Access::Allow => answer(&["allow_once", "allow_always"])?,
                    Access::Deny => answer(&["reject_once", "reject_always"])?,
                    Access::Ask => {
                        // The agent's own "don't ask again", if it has one.
                        let always = options.as_array().and_then(|options| {
                            let option = options
                                .iter()
                                .find(|option| option["kind"] == "allow_always")?;
                            Some(
                                option["name"]
                                    .as_str()
                                    .unwrap_or("Always allow")
                                    .to_string(),
                            )
                        });
                        let request = PermissionRequest {
                            id: ours.clone(),
                            title: call.asking(),
                            detail: call.detail(&self.directory),
                            kind: call.what(),
                            always,
                        };
                        let waiting = Waiting::Permission {
                            id,
                            options,
                            edit: None,
                        };
                        lock(&self.waiting).insert(ours, waiting);
                        return Ok(Some(AgentEvent::Permission(request)));
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
    /// The tool's own name, where the agent says it.
    name: String,
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
        if let Some(name) = text("name").or_else(|| {
            update["_meta"]["claudeCode"]["toolName"]
                .as_str()
                .map(str::to_string)
        }) {
            self.name = name;
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

    /// Which of the Map's tools this is, if it's one.
    fn map_tool(&self) -> Option<&'static str> {
        map::tool_of(&self.name).or_else(|| map::tool_of(&self.title))
    }

    /// One of Jig's own task tools, as the agent names it: Claude Code's
    /// `mcp__jig__create_task`, Gemini's `create_task (jig MCP Server)`.
    fn is_task_tool(&self) -> bool {
        let title = self.title.to_lowercase();
        title.contains(tasks::mcp::NAME)
            && tasks::mcp::TOOLS.iter().any(|tool| title.contains(tool))
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

    /// What it does, by its kind.
    fn what(&self) -> Asking {
        match self.kind.as_str() {
            "execute" => Asking::Command,
            "read" | "search" => Asking::Read,
            "fetch" => Asking::Web,
            _ => match self.raw_input["subagent_type"].as_str() {
                Some(kind) => Asking::Subagent {
                    explores: kind.eq_ignore_ascii_case("explore"),
                },
                None => Asking::Other,
            },
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

    /// A line for any finished call, for a subagent's list of what it did.
    fn summary(&self, directory: &str) -> String {
        if let Some(done) = self.done(directory) {
            return done;
        }
        let path = self.locations.first().map(|path| relative(path, directory));
        match (self.kind.as_str(), path) {
            ("read", Some(path)) => format!("Read {path}"),
            ("edit", Some(path)) => format!("Edited {path}"),
            _ if self.title.is_empty() => "Used a tool".into(),
            _ => self.title.clone(),
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
    /// Each subagent's words, by its session: the last is its report.
    subagents: HashMap<String, Vec<(Option<String>, String)>>,
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
            subagents: HashMap::new(),
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
            "tool_call" | "tool_call_update" => self.tool_call(update, None),
            "subagent_spawned" => {
                let text = |name: &str| update[name].as_str().unwrap_or_default().trim();
                let description = [text("name"), text("task")]
                    .into_iter()
                    .find(|text| !text.is_empty())
                    .unwrap_or("Subagent");
                vec![AgentEvent::Subagent {
                    id: text("subagentSessionId").to_string(),
                    description: description.to_string(),
                }]
            }
            "subagent_state_update" => {
                let id = update["subagentSessionId"].as_str().unwrap_or_default();
                let said = self.subagents.get(id).map(|messages| {
                    messages
                        .iter()
                        .rev()
                        .map(|(_, text)| text.trim())
                        .find(|text| !text.is_empty())
                        .unwrap_or_default()
                        .to_string()
                });
                let report = match (said, update["state"].as_str()) {
                    (Some(said), _) if !said.is_empty() => said,
                    (_, Some("failed")) => "It failed.".into(),
                    (_, Some("cancelled" | "disconnected")) => "It was stopped.".into(),
                    _ => "Done.".into(),
                };
                vec![AgentEvent::SubagentDone {
                    id: id.to_string(),
                    report,
                }]
            }
            _ => Vec::new(),
        }
    }

    /// An update from a subagent's session: its work goes under it, its
    /// words become its report, and a subagent it starts is one more.
    fn subagent_update(&mut self, session: &str, update: &Value) -> Vec<AgentEvent> {
        match update["sessionUpdate"].as_str().unwrap_or_default() {
            "agent_message_chunk" => {
                add_chunk(
                    self.subagents.entry(session.to_string()).or_default(),
                    update,
                );
                Vec::new()
            }
            "tool_call" | "tool_call_update" => self.tool_call(update, Some(session)),
            "subagent_spawned" | "subagent_state_update" => self.update(update),
            _ => Vec::new(),
        }
    }

    /// A tool call of the agent's, or of one of its subagents: a step while
    /// it runs, a line for the transcript once it's done. An edit made
    /// without asking is always the conversation's own line, so it shows.
    fn tool_call(&mut self, update: &Value, subagent: Option<&str>) -> Vec<AgentEvent> {
        let was_done = update["toolCallId"]
            .as_str()
            .and_then(|id| self.calls.get(id))
            .is_some_and(|call| call.status == "completed");
        let call = self.merge(update);
        match call.status.as_str() {
            "completed" if was_done => Vec::new(),
            "completed" if subagent.is_none() && call.map_tool().is_some() => {
                vec![AgentEvent::Drew {
                    tool: call.map_tool().unwrap_or_default().to_string(),
                    input: call.raw_input.clone(),
                }]
            }
            "completed" if call.kind == "edit" => {
                let asked = self.reviewed.contains(&call.id)
                    || call.content.iter().any(|content| {
                        content["path"]
                            .as_str()
                            .is_some_and(|path| self.written.iter().any(|w| w == Path::new(path)))
                    });
                match call.edited_path(&self.directory) {
                    Some(path) if !asked => {
                        vec![AgentEvent::Did(format!("Edited {path} without asking"))]
                    }
                    _ => Vec::new(),
                }
            }
            // A subagent's work is listed in full, reading included: it's
            // the only way to see what it did.
            "completed" => match subagent {
                Some(id) => vec![AgentEvent::SubagentDid {
                    id: id.to_string(),
                    text: call.summary(&self.directory),
                }],
                None => call
                    .done(&self.directory)
                    .map(AgentEvent::Did)
                    .into_iter()
                    .collect(),
            },
            "failed" => Vec::new(),
            _ => vec![AgentEvent::Step(call.step(&self.directory))],
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
    turns: Vec<super::Turn>,
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
        use super::Turn as Line;
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
                let was_done = call.status == "completed";
                call.merge(update);
                let call = call.clone();
                let line = match call.status.as_str() {
                    "completed" | "failed" if call.kind == "edit" => {
                        call.edited_path(&self.directory).map(|path| Line::Edit {
                            path,
                            accepted: call.status == "completed",
                        })
                    }
                    "completed" if call.map_tool().is_some() => Some(Line::Drew {
                        tool: call.map_tool().unwrap_or_default().to_string(),
                        input: call.raw_input.clone(),
                    }),
                    "completed" => call.done(&self.directory).map(Line::Did),
                    _ => None,
                };
                // Updates after the call is done change nothing.
                if was_done {
                    return;
                }
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
    fn finish(self) -> Vec<super::Turn> {
        use super::Turn as Line;
        self.turns
            .into_iter()
            .filter_map(|line| match line {
                Line::User(text) => Some(Line::User(super::asked(&text).to_string())),
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

    /// An agent that wants a sign-in: it answers `initialize` with a
    /// terminal login, and every `session/new` with "Authentication required".
    #[cfg(unix)]
    fn agent_wanting_a_sign_in() -> AgentCommandLine {
        let script = r#"
read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{},"authMethods":[{"id":"login","name":"Log in","description":"Use your account ","type":"terminal","args":["--cli","login"]},{"id":"browser","name":"Browser"}]}}'
read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Authentication required"}}'
read -r line
"#;
        AgentCommandLine {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into(), "agent".into()],
            env: Vec::new(),
            path: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_second_turn_is_refused_while_the_first_runs() {
        // Answers `initialize` and `session/new`, then never finishes a turn.
        let script = r#"
read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":1,"agentCapabilities":{}}}'
read -r line
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"sessionId":"s"}}'
read -r line
sleep 30
"#;
        let command = AgentCommandLine {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into(), "agent".into()],
            env: Vec::new(),
            path: None,
        };
        let connection = Connection::start("Fake", &command).unwrap();
        let session = Session::create(connection.clone(), Path::new("/"), None).unwrap();
        let request = AgentRequest {
            directory: "/".into(),
            prompt: "Hi".into(),
            model: String::new(),
            command: None,
            permissions: Permissions::default(),
        };
        let first = {
            let (session, request) = (session.clone(), request.clone());
            std::thread::spawn(move || session.run(&request, &|_| {}))
        };
        while !session.busy.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(5));
        }
        let error = session.run(&request, &|_| {}).unwrap_err();
        assert_eq!(
            error.to_string(),
            "Fake is still working on this conversation."
        );
        connection.stop();
        assert!(first.join().unwrap().is_err(), "the agent stopped");
        assert!(!session.busy.load(Ordering::SeqCst), "free once it ends");
    }

    #[cfg(unix)]
    #[test]
    fn says_when_the_agent_wants_a_sign_in_and_how() {
        let command = agent_wanting_a_sign_in();
        let connection = Connection::start("Fake", &command).unwrap();
        let error = match Session::create(connection, Path::new("/"), None) {
            Err(error) => error,
            Ok(_) => panic!("it wants a sign-in first"),
        };
        let needed = error.downcast_ref::<SignInNeeded>().expect("a sign-in");
        assert_eq!(needed.to_string(), "Fake needs you to sign in.");
        let [terminal, browser] = needed.methods.as_slice() else {
            panic!("two ways: {:?}", needed.methods);
        };
        assert_eq!(terminal.name, "Log in");
        assert_eq!(terminal.description, "Use your account");
        let login = terminal.terminal.as_ref().expect("in a terminal");
        assert_eq!(login.program, command.program);
        assert_eq!(
            login.args[3..],
            ["--cli", "login"],
            "after its own arguments"
        );
        assert_eq!(browser.terminal, None, "the agent does it itself");
    }

    #[test]
    fn finds_the_model_setting_among_the_options() {
        let options = json!([
            {"id": "mode", "category": "mode", "type": "select", "currentValue": "default",
             "options": [{"value": "default", "name": "Manual"}]},
            {"id": "model", "category": "model", "type": "select", "currentValue": "default",
             "options": [{"value": "default", "name": "Default (recommended)"},
                         {"value": "opus", "name": "Opus 5.5"}]},
        ]);
        let models = AgentModels::from_options(&options).unwrap();
        assert_eq!(models.config_id, "model");
        assert_eq!(models.current, "default");
        assert_eq!(models.options[1], ("opus".into(), "Opus 5.5".into()));

        let grouped = json!([{"id": "m", "category": "model", "type": "select", "currentValue": "a",
            "options": [{"group": "x", "name": "X", "options": [{"value": "a", "name": "A"}]}]}]);
        assert_eq!(
            AgentModels::from_options(&grouped).unwrap().options,
            [("a".into(), "A".into())]
        );
        assert_eq!(AgentModels::from_options(&json!([])), None);
    }

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
    fn a_drawing_on_the_map_is_reported_once_with_what_it_was_given() {
        let mut turn = TurnState::new("/p", Permissions::default());
        let started = turn.update(&json!({
            "sessionUpdate": "tool_call", "toolCallId": "d1", "status": "pending",
            "title": "mcp__jig-map__map_trace", "kind": "other",
            "rawInput": {"title": "Booking", "steps": [{"node": "a.rs#book", "label": "Book"}]},
        }));
        assert!(matches!(started.as_slice(), [AgentEvent::Step(_)]));
        let update =
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "d1", "status": "completed"});
        assert_eq!(
            turn.update(&update),
            [AgentEvent::Drew {
                tool: "map_trace".into(),
                input: json!({"title": "Booking", "steps": [{"node": "a.rs#book", "label": "Book"}]}),
            }]
        );
        assert!(turn.update(&update).is_empty(), "only once");
    }

    #[test]
    fn a_subagents_work_and_report_are_its_own() {
        let mut turn = TurnState::new("/p", Permissions::default());
        let spawned = turn.update(&json!({
            "sessionUpdate": "subagent_spawned", "subagentSessionId": "s1",
            "name": "Find the loader", "task": "Look for where config is read", "capabilities": {},
        }));
        assert_eq!(
            spawned,
            [AgentEvent::Subagent {
                id: "s1".into(),
                description: "Find the loader".into()
            }]
        );

        // Its reads and searches are listed under it, not the conversation.
        let read = turn.subagent_update(
            "s1",
            &json!({
                "sessionUpdate": "tool_call", "toolCallId": "r1", "kind": "read", "status": "completed",
                "title": "Read", "locations": [{"path": "/p/src/config.rs"}],
            }),
        );
        assert_eq!(
            read,
            [AgentEvent::SubagentDid {
                id: "s1".into(),
                text: "Read src/config.rs".into()
            }]
        );
        let ran = turn.subagent_update(
            "s1",
            &json!({
                "sessionUpdate": "tool_call", "toolCallId": "x1", "kind": "execute", "status": "completed",
                "rawInput": {"command": "rg load"},
            }),
        );
        assert_eq!(
            ran,
            [AgentEvent::SubagentDid {
                id: "s1".into(),
                text: "Ran `rg load`".into()
            }]
        );

        // Its words are its report, not the conversation's reply.
        let chunk = json!({"sessionUpdate": "agent_message_chunk", "messageId": "m1", "content": {"type": "text", "text": "It's in config.rs."}});
        turn.subagent_update("s1", &chunk);
        assert_eq!(turn.reply(), "");
        let done = turn.update(&json!({
            "sessionUpdate": "subagent_state_update", "subagentSessionId": "s1", "state": "completed",
        }));
        assert_eq!(
            done,
            [AgentEvent::SubagentDone {
                id: "s1".into(),
                report: "It's in config.rs.".into()
            }]
        );

        // An edit it makes without asking still shows in the conversation.
        let edited = turn.subagent_update(
            "s1",
            &json!({
                "sessionUpdate": "tool_call_update", "toolCallId": "e1", "kind": "edit", "status": "completed",
                "content": [{"type": "diff", "path": "/p/src/a.rs", "newText": "b"}],
            }),
        );
        assert_eq!(
            edited,
            [AgentEvent::Did("Edited src/a.rs without asking".into())]
        );
    }

    #[test]
    fn jigs_task_tools_are_known_by_name() {
        let call = |title: &str| ToolCall {
            title: title.into(),
            kind: "other".into(),
            ..ToolCall::default()
        };
        assert!(call("mcp__jig__create_task").is_task_tool());
        assert!(call("update_task (jig MCP Server)").is_task_tool());
        assert!(!call("mcp__other__create_task").is_task_tool());
        assert!(!call("mcp__jig__something_else").is_task_tool());
        assert_eq!(Permissions::default().tasks, Access::Allow);
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
        use super::Turn as Line;
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
