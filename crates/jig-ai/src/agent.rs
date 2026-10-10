//! Agent mode: conversations with a full coding agent (OpenCode, run as
//! `opencode serve`) instead of a single model call.
//!
//! The agent reads the project, uses its skills and commands and follows
//! `AGENTS.md` on its own. Jig keeps the last word on everything it does:
//! edits are set to "ask", so each one arrives as an [`EditRequest`] with a
//! diff before anything is written, and only reaches the disk once the user
//! accepts it. Shell commands, web access and subagents ask too, as a
//! [`PermissionRequest`]. Conversations stay in OpenCode, so Jig can list
//! them and pick one up again.

use std::cell::RefCell;
use std::collections::HashSet;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::config::{AgentModel, ProviderKind};
use crate::{PromptRequest, ProviderConfig};

/// A model OpenCode knows by itself, for trying the agent lane on its own.
pub const DEFAULT_MODEL: &str = "opencode-go/qwen3.8-flash";

/// Room OpenCode assumes for a model Jig hands it; it can't look these up
/// for a model it doesn't know.
const CONTEXT_TOKENS: u32 = 128_000;
const OUTPUT_TOKENS: u32 = 16_000;

/// What an agent run needs from OpenCode: the model, as OpenCode names it,
/// and the provider config to start the server with.
#[derive(Clone, Debug, PartialEq)]
pub struct AgentTarget {
    pub model: String,
    /// Passed as `OPENCODE_CONFIG_CONTENT`. `None` for a model OpenCode
    /// knows by itself.
    pub config: Option<Value>,
}

impl AgentTarget {
    /// Hand one of Jig's providers to OpenCode under a name of its own, so
    /// it never clashes with OpenCode's built-in providers or needs their
    /// model catalogue.
    pub fn new(agent: &AgentModel, api_key: Option<&str>) -> Self {
        let (provider, model) = match agent {
            AgentModel::OpenCode(model) => {
                return Self {
                    model: model.clone(),
                    config: None,
                };
            }
            AgentModel::Jig { provider, model } => (provider, model),
        };
        let id = format!("jig-{}", provider.name);
        Self {
            model: format!("{id}/{model}"),
            config: Some(
                json!({ "provider": { id: opencode_provider(provider, model, api_key) } }),
            ),
        }
    }
}

fn opencode_provider(provider: &ProviderConfig, model: &str, api_key: Option<&str>) -> Value {
    let npm = match provider.kind {
        ProviderKind::Anthropic => "@ai-sdk/anthropic",
        ProviderKind::Openai => "@ai-sdk/openai-compatible",
    };
    let mut options = json!({ "baseURL": provider.base_url.trim_end_matches('/') });
    if let Some(key) = api_key {
        options["apiKey"] = json!(key);
    }
    if let Some(header) = &provider.session_header {
        options["headers"] = json!({ header: crate::config::session_id() });
    }
    json!({
        "npm": npm,
        "name": provider.name,
        "options": options,
        "models": {
            model: {
                "name": model,
                "tool_call": true,
                "limit": { "context": CONTEXT_TOKENS, "output": OUTPUT_TOKENS },
            },
        },
    })
}

/// How the agent should behave inside Jig, added to its own system prompt.
const SYSTEM: &str = "You are running inside Jig, a code editor. The user talks to you from the code itself; a message may say which file they are in and what they selected. Make changes with your edit tools. Each edit is shown to the user, who accepts or rejects it; a rejection may come with a note, which you should follow. Shell commands, web access and subagents may wait for the user's go-ahead or be turned off; if one is turned down, carry on without it or ask. Change only what the task needs. When you are done, say briefly what you did; Markdown is fine. If the task is unclear, ask with your question tool or in your reply; later messages continue the same conversation.";

/// Whether the agent may use a tool: after asking the user each time, on
/// its own, or not at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    #[default]
    Ask,
    Allow,
    Deny,
}

impl Access {
    pub const ALL: [Access; 3] = [Access::Ask, Access::Allow, Access::Deny];

    /// As OpenCode and `settings.toml` write it.
    pub fn key(self) -> &'static str {
        match self {
            Access::Ask => "ask",
            Access::Allow => "allow",
            Access::Deny => "deny",
        }
    }

    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|access| access.key() == key)
            .unwrap_or_default()
    }
}

/// What the agent may do besides reading the project. Edits always wait
/// for review and nothing outside the project is touched; those aren't
/// choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Permissions {
    pub shell: Access,
    pub web_fetch: Access,
    pub web_search: Access,
    pub subagents: Access,
    /// Its question tool; without it, it asks in its reply.
    pub questions: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            shell: Access::Ask,
            web_fetch: Access::Ask,
            web_search: Access::Ask,
            subagents: Access::Ask,
            questions: true,
        }
    }
}

impl Permissions {
    /// The rules as OpenCode takes them for a session.
    fn rules(&self) -> Value {
        let questions = if self.questions {
            Access::Allow
        } else {
            Access::Deny
        };
        [
            ("edit", Access::Ask),
            ("bash", self.shell),
            ("task", self.subagents),
            ("webfetch", self.web_fetch),
            ("websearch", self.web_search),
            ("question", questions),
            ("external_directory", Access::Deny),
        ]
        .iter()
        .map(|(permission, access)| {
            json!({"permission": permission, "pattern": "*", "action": access.key()})
        })
        .collect()
    }
}

/// The longest title Jig gives a conversation, in characters.
const TITLE_CHARS: usize = 60;

/// A running `opencode serve`, private to this Jig process. It is stopped
/// when dropped.
pub struct AgentServer {
    url: String,
    /// `Authorization` header value; the server only answers to Jig.
    auth: String,
    http: ureq::Agent,
    child: Mutex<Child>,
}

impl AgentServer {
    /// Start the server with `config` added to the user's own OpenCode
    /// config, and wait until it listens.
    pub fn start(config: Option<&Value>) -> Result<Self> {
        let password = random_hex();
        let mut command = Command::new(opencode_binary());
        if let Some(config) = config {
            command.env("OPENCODE_CONFIG_CONTENT", config.to_string());
        }
        let mut child = command
            .args(["serve", "--port", "0", "--hostname", "127.0.0.1"])
            .env("OPENCODE_SERVER_PASSWORD", &password)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Couldn't start OpenCode. Is `opencode` installed?")?;
        let stdout = child.stdout.take().context("no stdout from opencode")?;
        let mut url = None;
        for line in BufReader::new(stdout).lines() {
            let line = line?;
            if let Some(rest) = line.split("listening on ").nth(1) {
                url = Some(rest.trim().trim_end_matches('/').to_string());
                break;
            }
        }
        let Some(url) = url else {
            let _ = child.kill();
            bail!("OpenCode exited before it started listening");
        };
        Ok(Self {
            url,
            auth: format!(
                "Basic {}",
                base64(format!("opencode:{password}").as_bytes())
            ),
            http: ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_connect(Some(Duration::from_secs(10)))
                .build()
                .into(),
            child: Mutex::new(child),
        })
    }

    fn request(
        &self,
        method: &str,
        path: &str,
        directory: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        self.request_with(method, path, directory, &[], body)
    }

    /// [`AgentServer::request`] with more query parameters.
    fn request_with(
        &self,
        method: &str,
        path: &str,
        directory: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Value> {
        let url = format!("{}{path}", self.url);
        let query = std::iter::once(("directory", directory)).chain(query.iter().copied());
        let mut response = match (method, body) {
            ("PATCH", Some(body)) => self
                .http
                .patch(&url)
                .query_pairs(query)
                .header("Authorization", &self.auth)
                .send_json(body),
            (_, Some(body)) => self
                .http
                .post(&url)
                .query_pairs(query)
                .header("Authorization", &self.auth)
                .send_json(body),
            ("GET", None) => self
                .http
                .get(&url)
                .query_pairs(query)
                .header("Authorization", &self.auth)
                .call(),
            (_, None) => self
                .http
                .post(&url)
                .query_pairs(query)
                .header("Authorization", &self.auth)
                .send_empty(),
        }
        .map_err(|error| anyhow!("Couldn't reach OpenCode: {error}"))?;
        let status = response.status();
        let text = response.body_mut().read_to_string()?;
        if !status.is_success() {
            bail!(
                "OpenCode {}: {}",
                status.as_u16(),
                text.chars().take(200).collect::<String>()
            );
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text).map_err(|error| anyhow!("Invalid JSON from OpenCode: {error}"))
    }

    /// Jig's earlier conversations in `directory`, latest first.
    pub fn conversations(&self, directory: &Path) -> Result<Vec<Conversation>> {
        let sessions = self.request_with(
            "GET",
            "/session",
            &canonical(directory),
            &[("roots", "true"), ("limit", "50")],
            None,
        )?;
        Ok(conversations(&sessions))
    }

    /// The commands and skills OpenCode offers in `directory`.
    pub fn commands(&self, directory: &Path) -> Result<Vec<AgentCommand>> {
        let commands = self.request("GET", "/command", &canonical(directory), None)?;
        Ok(commands
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|command| {
                Some(AgentCommand {
                    name: command["name"].as_str()?.to_string(),
                    description: command["description"]
                        .as_str()
                        .unwrap_or_default()
                        .lines()
                        .next()
                        .unwrap_or_default()
                        .to_string(),
                    skill: command["source"] == "skill",
                })
            })
            .collect())
    }
}

/// An earlier conversation with the agent, to pick up again.
#[derive(Clone, Debug, PartialEq)]
pub struct Conversation {
    pub id: String,
    /// What the user first asked, or `None` for conversations from before
    /// Jig named them.
    pub title: Option<String>,
    pub updated: SystemTime,
}

/// A command or skill OpenCode runs by name, as in "/review".
#[derive(Clone, Debug, PartialEq)]
pub struct AgentCommand {
    pub name: String,
    /// Its first line.
    pub description: String,
    pub skill: bool,
}

/// The sessions Jig started, which are the ones whose edits ask first:
/// the user's own OpenCode sessions don't, so they aren't safe to continue
/// here unchanged.
fn conversations(sessions: &Value) -> Vec<Conversation> {
    let mut found: Vec<Conversation> = sessions
        .as_array()
        .into_iter()
        .flatten()
        .filter(|session| session["parentID"].is_null() && session["time"]["archived"].is_null())
        .filter(|session| {
            session["metadata"]["jig"] == true
                || session["permission"].as_array().is_some_and(|rules| {
                    rules
                        .iter()
                        .any(|rule| rule["permission"] == "edit" && rule["action"] == "ask")
                })
        })
        .filter_map(|session| {
            let title = session["title"].as_str().unwrap_or_default().trim();
            Some(Conversation {
                id: session["id"].as_str()?.to_string(),
                title: (!title.is_empty() && title != "Jig").then(|| title.to_string()),
                updated: SystemTime::UNIX_EPOCH
                    + Duration::from_millis(session["time"]["updated"].as_u64().unwrap_or(0)),
            })
        })
        .collect();
    found.sort_by_key(|conversation| std::cmp::Reverse(conversation.updated));
    found
}

fn canonical(directory: &Path) -> String {
    directory
        .canonicalize()
        .unwrap_or_else(|_| directory.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// A conversation's title: the start of what was first asked.
pub fn title(asked: &str) -> String {
    let line = asked.lines().next().unwrap_or_default().trim();
    if line.chars().count() <= TITLE_CHARS {
        line.to_string()
    } else {
        let cut: String = line.chars().take(TITLE_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

impl AgentServer {
    /// Stop the server process. Sessions still running on it fail.
    pub fn stop(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for AgentServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The message for the agent: the instruction, the user's note, where they
/// are in the file and what they selected. OpenCode finds `AGENTS.md`,
/// skills and commands on its own.
pub fn prompt(request: &PromptRequest) -> String {
    let mut out = request.instruction.trim().to_string();
    if let Some(note) = &request.comment {
        out.push_str(&format!("\nNote: {note}"));
    }
    if !request.diagnostics.is_empty() {
        out.push_str("\nThe language server reports:");
        for diagnostic in &request.diagnostics {
            out.push_str(&format!("\n- {}", diagnostic.describe()));
        }
    }
    let file = request.file_name.as_deref().unwrap_or("an unsaved file");
    let line = |offset: usize| request.text[..offset].matches('\n').count() + 1;
    let range = request.target.clone();
    out.push_str("\n\n");
    if range.is_empty() {
        out.push_str(&format!(
            "I'm in {file}, with the cursor on line {}. New code goes there.",
            line(range.start)
        ));
    } else if range == (0..request.text.len()) {
        out.push_str(&format!("I'm in {file}; this is about the whole file."));
    } else {
        let (first, last) = (line(range.start), line(range.end.saturating_sub(1)));
        let lines = if first == last {
            format!("line {first}")
        } else {
            format!("lines {first}-{last}")
        };
        out.push_str(&format!(
            "I'm in {file}, with {lines} selected:\n```{}\n{}\n```",
            request.language,
            request.target_text().trim_end_matches('\n')
        ));
    }
    out
}

/// What the user wrote, without the note on where they are that
/// [`prompt`] adds.
pub fn asked(prompt: &str) -> &str {
    prompt.split("\n\nI'm in ").next().unwrap_or(prompt).trim()
}

/// What the agent is asked to do.
#[derive(Clone, Debug)]
pub struct AgentRequest {
    /// The project folder the agent works in.
    pub directory: PathBuf,
    /// The message, or a command's arguments.
    pub prompt: String,
    /// `provider/model`, as OpenCode names them.
    pub model: String,
    /// Run this OpenCode command or skill, e.g. "review", with `prompt` as
    /// its arguments.
    pub command: Option<String>,
    /// What it may do this turn.
    pub permissions: Permissions,
}

/// One edit the agent wants to make, waiting for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditRequest {
    pub id: String,
    pub path: PathBuf,
    /// A unified diff against the file on disk.
    pub diff: String,
}

/// Something else the agent wants to do that waits for the user: run a
/// command, fetch a page, start a subagent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRequest {
    pub id: String,
    /// What it wants to do, e.g. "Run a command".
    pub title: String,
    /// The command, address or task.
    pub detail: String,
}

/// Questions the agent asks with its question tool, answered all at once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionRequest {
    pub id: String,
    pub questions: Vec<Question>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub question: String,
    /// The labels of its choices; the user may also answer in words.
    pub options: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    /// A short line for the UI, e.g. "Reading src/lib.rs".
    Step(String),
    Edit(EditRequest),
    Permission(PermissionRequest),
    Question(QuestionRequest),
    /// Something it did that belongs in the transcript, e.g. a command it
    /// ran: "Ran `cargo test`".
    Did(String),
    /// The agent finished; its closing words.
    Done(String),
}

/// One line of an earlier conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Turn {
    User(String),
    Agent(String),
    Edit { path: String, accepted: bool },
    Did(String),
}

/// One conversation with the agent. Cheap to clone, so the UI can reply to
/// edits and abort while another thread runs [`AgentSession::run`].
#[derive(Clone)]
pub struct AgentSession {
    server: Arc<AgentServer>,
    id: String,
    directory: String,
}

impl AgentSession {
    /// A new conversation, titled with what the user asked.
    pub fn create(
        server: Arc<AgentServer>,
        directory: &Path,
        title: &str,
        permissions: &Permissions,
    ) -> Result<Self> {
        let directory = canonical(directory);
        let session = server.request(
            "POST",
            "/session",
            &directory,
            Some(&json!({
                "title": self::title(title),
                "permission": permissions.rules(),
                "metadata": {"jig": true},
            })),
        )?;
        let id = session["id"]
            .as_str()
            .context("OpenCode didn't return a session id")?
            .to_string();
        Ok(Self {
            server,
            id,
            directory,
        })
    }

    /// Pick up an earlier conversation. Its next turn runs under the
    /// permissions of the day.
    pub fn open(server: Arc<AgentServer>, directory: &Path, id: &str) -> Result<Self> {
        let directory = canonical(directory);
        Ok(Self {
            server,
            id: id.to_string(),
            directory,
        })
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// The conversation so far.
    pub fn history(&self) -> Result<Vec<Turn>> {
        let messages = self.server.request(
            "GET",
            &format!("/session/{}/message", self.id),
            &self.directory,
            None,
        )?;
        Ok(history(&messages, &self.directory))
    }

    /// Send `request` and report what happens until the agent is done.
    /// Blocks; edits and other requests wait until [`AgentSession::reply`]
    /// or [`AgentSession::answer`] is called.
    pub fn run(&self, request: &AgentRequest, on_event: &dyn Fn(AgentEvent)) -> Result<()> {
        // Subscribe before prompting, so no event is missed.
        let events = self
            .server
            .http
            .get(format!("{}/event", self.server.url))
            .query("directory", &self.directory)
            .header("Authorization", &self.server.auth)
            .call()
            .map_err(|error| anyhow!("Couldn't reach OpenCode: {error}"))?;
        let (provider, model) = request.model.split_once('/').with_context(|| {
            format!(
                "agent model {:?} should look like provider/model",
                request.model
            )
        })?;
        // Settings may have changed since the last turn, or since an
        // earlier conversation was had.
        self.server.request(
            "PATCH",
            &format!("/session/{}", self.id),
            &self.directory,
            Some(&json!({"permission": request.permissions.rules()})),
        )?;
        // A command's request only returns once it's done, so it's sent
        // from another thread; a failure there ends the run here.
        let failed: Arc<Mutex<Option<String>>> = Arc::default();
        match &request.command {
            Some(command) => {
                let (session, failed) = (self.clone(), failed.clone());
                let body = json!({
                    "command": command,
                    "arguments": request.prompt,
                    "model": request.model,
                });
                std::thread::spawn(move || {
                    let path = format!("/session/{}/command", session.id);
                    if let Err(error) =
                        session
                            .server
                            .request("POST", &path, &session.directory, Some(&body))
                    {
                        *failed.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(format!("{error:#}"));
                    }
                });
            }
            None => {
                self.server.request(
                    "POST",
                    &format!("/session/{}/prompt_async", self.id),
                    &self.directory,
                    Some(&json!({
                        "model": {"providerID": provider, "modelID": model},
                        "system": SYSTEM,
                        "parts": [{"type": "text", "text": request.prompt}],
                    })),
                )?;
            }
        }

        let sessions = RefCell::new(HashSet::from([self.id.clone()]));
        let reader = BufReader::new(events.into_body().into_reader());
        for line in reader.lines() {
            let line = line.context("lost the connection to OpenCode")?;
            if let Some(error) = failed.lock().unwrap_or_else(|e| e.into_inner()).take() {
                bail!("{error}");
            }
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            let Ok(event) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            match self.interpret(&event, &sessions)? {
                Some(AgentEvent::Done(_)) => {
                    on_event(AgentEvent::Done(self.last_reply()?));
                    return Ok(());
                }
                Some(event) => on_event(event),
                None => {}
            }
        }
        bail!("OpenCode closed the connection")
    }

    /// Turn one server event into something the UI cares about. Events of
    /// the subagents it starts count too: their edits wait for the user
    /// like its own.
    fn interpret(
        &self,
        event: &Value,
        sessions: &RefCell<HashSet<String>>,
    ) -> Result<Option<AgentEvent>> {
        let properties = &event["properties"];
        let kind = event["type"].as_str().unwrap_or_default();
        if matches!(kind, "session.created" | "session.updated")
            && let (Some(id), Some(parent)) = (
                properties["info"]["id"].as_str(),
                properties["info"]["parentID"].as_str(),
            )
            && sessions.borrow().contains(parent)
        {
            sessions.borrow_mut().insert(id.to_string());
        }
        let session = properties["sessionID"]
            .as_str()
            .or_else(|| properties["part"]["sessionID"].as_str());
        if !session.is_some_and(|session| sessions.borrow().contains(session)) {
            return Ok(None);
        }
        let ours = session == Some(self.id.as_str());
        Ok(match kind {
            "session.idle" if ours => Some(AgentEvent::Done(String::new())),
            "session.error" if ours => {
                let error = &properties["error"];
                // Stopping the agent isn't a failure.
                if error["name"] == "MessageAbortedError" {
                    return Ok(None);
                }
                let message = error["data"]["message"]
                    .as_str()
                    .or_else(|| error["name"].as_str())
                    .unwrap_or("the agent failed");
                bail!("{message}")
            }
            "permission.asked" if properties["permission"] == "edit" => {
                let metadata = &properties["metadata"];
                Some(AgentEvent::Edit(EditRequest {
                    id: properties["id"].as_str().unwrap_or_default().to_string(),
                    path: PathBuf::from(metadata["filepath"].as_str().unwrap_or_default()),
                    diff: metadata["diff"].as_str().unwrap_or_default().to_string(),
                }))
            }
            "permission.asked" => match permission(properties) {
                Some(request) => Some(AgentEvent::Permission(request)),
                None => {
                    // Anything else Jig can't show: say no, so the agent moves on.
                    let id = properties["id"].as_str().unwrap_or_default();
                    self.reply(id, false, Some("That isn't available inside Jig."))?;
                    None
                }
            },
            "question.asked" => Some(AgentEvent::Question(QuestionRequest {
                id: properties["id"].as_str().unwrap_or_default().to_string(),
                questions: properties["questions"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|question| Question {
                        question: question["question"].as_str().unwrap_or_default().into(),
                        options: question["options"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|option| option["label"].as_str().map(str::to_string))
                            .collect(),
                    })
                    .collect(),
            })),
            "message.part.updated" => {
                let part = &properties["part"];
                let tool = part["tool"].as_str().unwrap_or_default();
                match part["state"]["status"].as_str() {
                    Some("running") if part["type"] == "tool" => Some(AgentEvent::Step(describe(
                        tool,
                        &part["state"]["input"],
                        &self.directory,
                    ))),
                    Some("completed") if part["type"] == "tool" => {
                        did(tool, &part["state"]["input"]).map(AgentEvent::Did)
                    }
                    _ => None,
                }
            }
            _ => None,
        })
    }

    /// The agent's closing words: the text of its last message.
    fn last_reply(&self) -> Result<String> {
        let messages = self.server.request(
            "GET",
            &format!("/session/{}/message", self.id),
            &self.directory,
            None,
        )?;
        let last = messages
            .as_array()
            .into_iter()
            .flatten()
            .rev()
            .find(|message| message["info"]["role"] == "assistant");
        Ok(last.map(text_of).unwrap_or_default())
    }

    /// Let an edit or other request through, or turn it down with an
    /// optional note the agent sees.
    pub fn reply(&self, request_id: &str, accept: bool, note: Option<&str>) -> Result<()> {
        let mut body = json!({"reply": if accept { "once" } else { "reject" }});
        if let Some(note) = note {
            body["message"] = json!(note);
        }
        self.server
            .request(
                "POST",
                &format!("/permission/{request_id}/reply"),
                &self.directory,
                Some(&body),
            )
            .map(|_| ())
    }

    /// Answer the agent's questions, one answer each, in order; `None`
    /// declines them.
    pub fn answer(&self, question_id: &str, answers: Option<&[String]>) -> Result<()> {
        let (path, body) = match answers {
            Some(answers) => (
                format!("/question/{question_id}/reply"),
                Some(json!({
                    "answers": answers.iter().map(|answer| vec![answer]).collect::<Vec<_>>()
                })),
            ),
            None => (format!("/question/{question_id}/reject"), None),
        };
        self.server
            .request("POST", &path, &self.directory, body.as_ref())
            .map(|_| ())
    }

    /// Stop the agent. [`AgentSession::run`] then returns.
    pub fn abort(&self) -> Result<()> {
        self.server
            .request(
                "POST",
                &format!("/session/{}/abort", self.id),
                &self.directory,
                None,
            )
            .map(|_| ())
    }
}

/// The text a message says, paragraph by paragraph.
fn text_of(message: &Value) -> String {
    message["parts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|part| part["type"] == "text" && part["synthetic"] != true)
        .filter_map(|part| part["text"].as_str())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// A conversation's messages as the transcript shows them.
fn history(messages: &Value, directory: &str) -> Vec<Turn> {
    let mut turns = Vec::new();
    for message in messages.as_array().into_iter().flatten() {
        if message["info"]["role"] == "user" {
            let text = text_of(message);
            if !text.is_empty() {
                turns.push(Turn::User(asked(&text).to_string()));
            }
            continue;
        }
        for part in message["parts"].as_array().into_iter().flatten() {
            match part["type"].as_str() {
                Some("text") if part["synthetic"] != true => {
                    let text = part["text"].as_str().unwrap_or_default().trim();
                    if !text.is_empty() {
                        turns.push(Turn::Agent(text.to_string()));
                    }
                }
                Some("tool") => {
                    let tool = part["tool"].as_str().unwrap_or_default();
                    let state = &part["state"];
                    let completed = state["status"] == "completed";
                    if is_edit(tool)
                        && matches!(state["status"].as_str(), Some("completed" | "error"))
                    {
                        turns.push(Turn::Edit {
                            path: relative(&path_of(&state["input"]), directory),
                            accepted: completed,
                        });
                    } else if completed && let Some(text) = did(tool, &state["input"]) {
                        turns.push(Turn::Did(text));
                    }
                }
                _ => {}
            }
        }
    }
    turns
}

/// A request that isn't an edit, as the user is asked it. `None` for
/// requests Jig doesn't ask about.
fn permission(properties: &Value) -> Option<PermissionRequest> {
    let metadata = &properties["metadata"];
    let pattern = properties["patterns"]
        .as_array()
        .and_then(|patterns| patterns.first())
        .and_then(Value::as_str)
        .unwrap_or_default();
    let field = |name: &str| metadata[name].as_str().map(str::to_string);
    let (title, detail) = match properties["permission"].as_str()? {
        "bash" => (
            "Run a command",
            field("command").unwrap_or_else(|| pattern.into()),
        ),
        "webfetch" => (
            "Fetch a page",
            field("url").unwrap_or_else(|| pattern.into()),
        ),
        "websearch" => (
            "Search the web",
            field("query").unwrap_or_else(|| pattern.into()),
        ),
        "task" => (
            "Start a subagent",
            field("description").unwrap_or_else(|| pattern.into()),
        ),
        "doom_loop" => ("Repeat the same step again", pattern.into()),
        _ => return None,
    };
    Some(PermissionRequest {
        id: properties["id"].as_str()?.to_string(),
        title: title.into(),
        detail,
    })
}

fn is_edit(tool: &str) -> bool {
    matches!(
        tool,
        "edit" | "write" | "patch" | "multiedit" | "apply_patch"
    )
}

/// The transcript's line for a tool call worth keeping, once it's done.
/// Reading and searching are left out; edits have lines of their own.
fn did(tool: &str, input: &Value) -> Option<String> {
    let field = |name: &str| input[name].as_str().unwrap_or_default().trim().to_string();
    Some(match tool {
        "bash" => format!("Ran `{}`", field("command")),
        "webfetch" => format!("Fetched {}", field("url")),
        "websearch" => format!("Searched the web for {}", field("query")),
        "task" => format!("Subagent: {}", field("description")),
        "skill" => format!("Used skill {}", field("name")),
        _ => return None,
    })
}

fn path_of(input: &Value) -> String {
    input["filePath"]
        .as_str()
        .or_else(|| input["path"].as_str())
        .unwrap_or("")
        .to_string()
}

fn relative(path: &str, directory: &str) -> String {
    let relative = path
        .strip_prefix(directory)
        .map(|rest| rest.trim_start_matches('/'))
        .unwrap_or(path);
    if relative.is_empty() {
        ".".to_string()
    } else {
        relative.to_string()
    }
}

/// A short, human-readable line for a tool call.
fn describe(tool: &str, input: &Value, directory: &str) -> String {
    let path = || relative(&path_of(input), directory);
    match tool {
        "read" => format!("Reading {}", path()),
        "list" => format!("Listing {}", path()),
        tool if is_edit(tool) => format!("Editing {}", path()),
        "glob" | "grep" => format!(
            "Searching for {}",
            input["pattern"].as_str().unwrap_or("files")
        ),
        "skill" => format!("Using skill {}", input["name"].as_str().unwrap_or("")),
        "todowrite" | "todoread" => "Planning".into(),
        "bash" => "Running a command".into(),
        "webfetch" | "websearch" => "Looking on the web".into(),
        "task" => "Working with a subagent".into(),
        "question" => "Asking".into(),
        other => format!("Running {other}"),
    }
}

/// The `opencode` binary: on `PATH`, or where its installer puts it. An app
/// started from Finder doesn't see the shell's `PATH`.
fn opencode_binary() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        let installed = Path::new(&home).join(".opencode/bin/opencode");
        if installed.exists() {
            return installed;
        }
    }
    PathBuf::from("opencode")
}

fn random_hex() -> String {
    use std::hash::{BuildHasher as _, RandomState};
    (0..2)
        .map(|i| format!("{:016x}", RandomState::new().hash_one(i)))
        .collect()
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Apply a unified diff to `old`. Each hunk is looked for where its header
/// says, and anywhere later in the file if the text has moved.
pub fn apply_diff(old: &str, diff: &str) -> Result<String> {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let mut out = String::new();
    let mut cursor = 0;
    let mut lines = diff.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let Some(header) = line.strip_prefix("@@ -") else {
            continue;
        };
        let start: usize = header
            .split([',', ' '])
            .next()
            .and_then(|n| n.parse().ok())
            .context("malformed hunk header")?;
        // Old and new text of the hunk, line by line.
        let (mut before, mut after): (Vec<String>, Vec<String>) = (vec![], vec![]);
        // Which side the last line went to, for "\ No newline at end of file".
        let mut last = (false, false);
        while let Some(&body) = lines.peek() {
            if body.starts_with("@@") {
                break;
            }
            lines.next();
            let (kind, text) = body.split_at(body.len().min(1));
            match kind {
                " " => {
                    before.push(text.into());
                    after.push(text.into());
                    last = (true, true);
                }
                "-" => {
                    before.push(text.into());
                    last = (true, false);
                }
                "+" => {
                    after.push(text.into());
                    last = (false, true);
                }
                "\\" => {
                    if last.0
                        && let Some(line) = before.last_mut()
                    {
                        line.truncate(line.trim_end_matches('\n').len());
                    }
                    if last.1
                        && let Some(line) = after.last_mut()
                    {
                        line.truncate(line.trim_end_matches('\n').len());
                    }
                }
                _ => {}
            }
        }
        let fits = |at: usize| {
            at + before.len() <= old_lines.len()
                && before
                    .iter()
                    .zip(&old_lines[at..])
                    .all(|(a, b)| a.trim_end_matches('\n') == b.trim_end_matches('\n'))
        };
        let expected = start.saturating_sub(1).max(cursor);
        let at = if fits(expected) {
            expected
        } else {
            (cursor..=old_lines.len())
                .find(|&at| fits(at))
                .context("the file changed since the agent read it")?
        };
        out.extend(old_lines[cursor..at].iter().copied());
        out.extend(after.iter().map(String::as_str));
        cursor = at + before.len();
    }
    out.extend(old_lines[cursor..].iter().copied());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_an_opencode_diff() {
        let old = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
        let diff = "Index: /p/lib.rs\n===================================================================\n--- /p/lib.rs\n+++ /p/lib.rs\n@@ -1,3 +1,4 @@\n+/// Adds two integers.\n fn add(a: i32, b: i32) -> i32 {\n     a + b\n }\n";
        assert_eq!(
            apply_diff(old, diff).unwrap(),
            "/// Adds two integers.\nfn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n"
        );
    }

    #[test]
    fn applies_several_hunks_and_removals() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\n";
        let diff = "--- x\n+++ x\n@@ -1,2 +1,2 @@\n-a\n+A\n b\n@@ -7,2 +7,1 @@\n g\n-h\n";
        assert_eq!(apply_diff(old, diff).unwrap(), "A\nb\nc\nd\ne\nf\ng\n");
    }

    #[test]
    fn handles_a_missing_final_newline() {
        let old = "one\ntwo";
        let diff = "--- x\n+++ x\n@@ -1,2 +1,2 @@\n one\n-two\n\\ No newline at end of file\n+three\n\\ No newline at end of file\n";
        assert_eq!(apply_diff(old, diff).unwrap(), "one\nthree");
    }

    #[test]
    fn finds_a_hunk_that_moved() {
        let old = "x\ny\na\nb\n";
        let diff = "--- x\n+++ x\n@@ -1,2 +1,2 @@\n a\n-b\n+B\n";
        assert_eq!(apply_diff(old, diff).unwrap(), "x\ny\na\nB\n");
    }

    #[test]
    fn refuses_a_diff_that_doesnt_fit() {
        let diff = "--- x\n+++ x\n@@ -1,1 +1,1 @@\n-nope\n+yes\n";
        assert!(apply_diff("something else\n", diff).is_err());
    }

    #[test]
    fn describes_tool_calls_relative_to_the_project() {
        let input = json!({"filePath": "/p/src/lib.rs"});
        assert_eq!(describe("read", &input, "/p"), "Reading src/lib.rs");
        assert_eq!(
            describe("grep", &json!({"pattern": "fn main"}), "/p"),
            "Searching for fn main"
        );
    }

    #[test]
    fn prompt_says_where_the_user_is() {
        let text = "fn a() {}\nfn b() {}\nfn c() {}\n".to_string();
        let request = PromptRequest {
            instruction: "Add docs".into(),
            comment: Some("terse".into()),
            project_rules: None,
            language: "rust".into(),
            file_name: Some("src/lib.rs".into()),
            target: 10..20,
            text,
            diagnostics: Vec::new(),
        };
        assert_eq!(
            prompt(&request),
            "Add docs\nNote: terse\n\nI'm in src/lib.rs, with line 2 selected:\n```rust\nfn b() {}\n```"
        );
        let at_cursor = PromptRequest {
            target: 20..20,
            comment: None,
            ..request
        };
        assert!(prompt(&at_cursor).ends_with("cursor on line 3. New code goes there."));
        let with_problems = PromptRequest {
            diagnostics: vec![crate::prompt::Diagnostic {
                line: 3,
                severity: "error".into(),
                message: "expected `;`".into(),
            }],
            ..at_cursor
        };
        assert!(prompt(&with_problems).starts_with(
            "Add docs\nThe language server reports:\n- line 3, error: expected `;`\n\n"
        ));
    }

    #[test]
    fn lists_only_the_conversations_jig_started() {
        let sessions = json!([
            {"id": "ses_old", "title": "Jig", "time": {"updated": 1000},
             "permission": [{"permission": "edit", "pattern": "*", "action": "ask"}]},
            {"id": "ses_new", "title": "Add docs", "metadata": {"jig": true}, "time": {"updated": 2000}},
            {"id": "ses_tui", "title": "Mine", "time": {"updated": 3000}, "permission": []},
            {"id": "ses_child", "parentID": "ses_new", "metadata": {"jig": true}, "time": {"updated": 4000}},
            {"id": "ses_gone", "metadata": {"jig": true}, "time": {"updated": 5000, "archived": 5000}},
        ]);
        let found = conversations(&sessions);
        let ids: Vec<_> = found.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["ses_new", "ses_old"], "latest first");
        assert_eq!(found[0].title.as_deref(), Some("Add docs"));
        assert_eq!(found[1].title, None, "untitled from before");
    }

    #[test]
    fn history_keeps_what_was_said_and_done() {
        let messages = json!([
            {"info": {"role": "user"}, "parts": [
                {"type": "text", "text": "Add docs\n\nI'm in src/lib.rs, with line 2 selected:\n```rust\nfn b() {}\n```"},
            ]},
            {"info": {"role": "assistant"}, "parts": [
                {"type": "step-start"},
                {"type": "tool", "tool": "read", "state": {"status": "completed", "input": {"filePath": "/p/src/lib.rs"}}},
                {"type": "tool", "tool": "bash", "state": {"status": "completed", "input": {"command": "cargo test"}}},
                {"type": "tool", "tool": "edit", "state": {"status": "completed", "input": {"filePath": "/p/src/lib.rs"}}},
                {"type": "tool", "tool": "edit", "state": {"status": "error", "input": {"filePath": "/p/src/main.rs"}}},
                {"type": "text", "text": "Documented `b`."},
            ]},
        ]);
        assert_eq!(
            history(&messages, "/p"),
            [
                Turn::User("Add docs".into()),
                Turn::Did("Ran `cargo test`".into()),
                Turn::Edit {
                    path: "src/lib.rs".into(),
                    accepted: true
                },
                Turn::Edit {
                    path: "src/main.rs".into(),
                    accepted: false
                },
                Turn::Agent("Documented `b`.".into()),
            ]
        );
    }

    #[test]
    fn asks_about_commands_and_the_web_but_not_the_rest() {
        let bash = json!({"id": "per_1", "permission": "bash", "patterns": ["cargo test"], "metadata": {}});
        assert_eq!(
            permission(&bash),
            Some(PermissionRequest {
                id: "per_1".into(),
                title: "Run a command".into(),
                detail: "cargo test".into(),
            })
        );
        let fetch = json!({"id": "per_2", "permission": "webfetch", "patterns": [], "metadata": {"url": "https://x.dev"}});
        assert_eq!(permission(&fetch).unwrap().detail, "https://x.dev");
        let other = json!({"id": "per_3", "permission": "external_directory", "patterns": ["/etc"], "metadata": {}});
        assert_eq!(permission(&other), None);
    }

    #[test]
    fn permissions_become_opencode_rules() {
        let permissions = Permissions {
            shell: Access::Allow,
            web_search: Access::Deny,
            questions: false,
            ..Permissions::default()
        };
        let rules = permissions.rules();
        let action = |name: &str| {
            rules
                .as_array()
                .unwrap()
                .iter()
                .find(|rule| rule["permission"] == name)
                .map(|rule| rule["action"].as_str().unwrap().to_string())
        };
        assert_eq!(action("bash").as_deref(), Some("allow"));
        assert_eq!(action("websearch").as_deref(), Some("deny"));
        assert_eq!(action("webfetch").as_deref(), Some("ask"));
        assert_eq!(action("question").as_deref(), Some("deny"));
        assert_eq!(action("edit").as_deref(), Some("ask"), "always");
        assert_eq!(
            action("external_directory").as_deref(),
            Some("deny"),
            "always"
        );
    }

    #[test]
    fn titles_are_the_start_of_what_was_asked() {
        assert_eq!(title("Add docs\nmore"), "Add docs");
        let long = "word ".repeat(30);
        assert_eq!(title(&long).chars().count(), TITLE_CHARS);
        assert!(title(&long).ends_with('…'));
    }

    #[test]
    fn base64_matches_known_values() {
        assert_eq!(base64(b"opencode:pw"), "b3BlbmNvZGU6cHc=");
        assert_eq!(base64(b"ab"), "YWI=");
    }

    #[test]
    fn jig_providers_are_handed_to_opencode_under_their_own_name() {
        let config = crate::Config::parse(
            "[[provider]]\nname = \"openrouter\"\nkind = \"openai\"\nbase_url = \"https://openrouter.ai/api/v1/\"\n",
        )
        .unwrap();
        let agent = AgentModel::Jig {
            provider: config.providers[0].clone(),
            model: "anthropic/claude-sonnet-5-5".into(),
        };
        let target = AgentTarget::new(&agent, Some("sk"));
        assert_eq!(target.model, "jig-openrouter/anthropic/claude-sonnet-5-5");
        let provider = &target.config.unwrap()["provider"]["jig-openrouter"];
        assert_eq!(provider["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(
            provider["options"]["baseURL"],
            "https://openrouter.ai/api/v1"
        );
        assert_eq!(provider["options"]["apiKey"], "sk");
        assert_eq!(
            provider["models"]["anthropic/claude-sonnet-5-5"]["tool_call"],
            true
        );

        let native = AgentTarget::new(&AgentModel::OpenCode(DEFAULT_MODEL.into()), None);
        assert_eq!(
            native,
            AgentTarget {
                model: DEFAULT_MODEL.into(),
                config: None
            }
        );
    }
}
