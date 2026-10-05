//! The agent lane: commands handed to a full coding agent (OpenCode, run as
//! `opencode serve`) instead of a single model call.
//!
//! The agent reads the project, uses its skills and follows `AGENTS.md` on
//! its own. Jig keeps the last word on every change: edits are set to "ask",
//! so each one arrives as an [`EditRequest`] with a diff before anything is
//! written, and only reaches the disk once the user accepts it. Shell
//! commands, web access and subagents are turned off.

use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

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
const SYSTEM: &str = "You are running inside Jig, a code editor. The user gave you this task from the code itself; the message says which file they are in and what they selected. Make the change with your edit tools. Each edit is shown to the user, who accepts or rejects it; a rejection may come with a note, which you should follow. Change only what the task needs. When you are done, reply with one plain sentence of at most 20 words saying what you did. If the task is unclear, ask one short question instead; the user can reply, and later messages continue the same task. No markdown, no lists.";

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
        let url = format!("{}{path}", self.url);
        let query = [("directory", directory)];
        let mut response = match body {
            Some(body) => self
                .http
                .post(&url)
                .query_pairs(query)
                .header("Authorization", &self.auth)
                .send_json(body),
            None if method == "GET" => self
                .http
                .get(&url)
                .query_pairs(query)
                .header("Authorization", &self.auth)
                .call(),
            None => self
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

/// What the agent is asked to do.
#[derive(Clone, Debug)]
pub struct AgentRequest {
    /// The project folder the agent works in.
    pub directory: PathBuf,
    pub prompt: String,
    /// `provider/model`, as OpenCode names them.
    pub model: String,
}

/// One edit the agent wants to make, waiting for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditRequest {
    pub id: String,
    pub path: PathBuf,
    /// A unified diff against the file on disk.
    pub diff: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    /// A short line for the UI, e.g. "Reading src/lib.rs".
    Step(String),
    Edit(EditRequest),
    /// The agent finished; its closing sentence.
    Done(String),
}

/// One conversation with the agent. Cheap to clone, so the UI can reply to
/// edits and abort while another thread runs [`AgentSession::run`].
#[derive(Clone)]
pub struct AgentSession {
    server: std::sync::Arc<AgentServer>,
    id: String,
    directory: String,
}

impl AgentSession {
    pub fn create(server: std::sync::Arc<AgentServer>, directory: &Path) -> Result<Self> {
        let directory = directory
            .canonicalize()
            .unwrap_or_else(|_| directory.to_path_buf())
            .to_string_lossy()
            .into_owned();
        let rules: Vec<Value> = [
            ("edit", "ask"),
            ("bash", "deny"),
            ("task", "deny"),
            ("webfetch", "deny"),
            ("websearch", "deny"),
            ("question", "deny"),
            ("external_directory", "deny"),
        ]
        .iter()
        .map(|(permission, action)| json!({"permission": permission, "pattern": "*", "action": action}))
        .collect();
        let session = server.request(
            "POST",
            "/session",
            &directory,
            Some(&json!({"title": "Jig", "permission": rules})),
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

    /// Send `request` and report what happens until the agent is done.
    /// Blocks; edits wait until [`AgentSession::reply`] is called.
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

        let reader = BufReader::new(events.into_body().into_reader());
        for line in reader.lines() {
            let line = line.context("lost the connection to OpenCode")?;
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            let Ok(event) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            match self.interpret(&event)? {
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

    /// Turn one server event into something the UI cares about.
    fn interpret(&self, event: &Value) -> Result<Option<AgentEvent>> {
        let properties = &event["properties"];
        let session = properties["sessionID"]
            .as_str()
            .or_else(|| properties["part"]["sessionID"].as_str());
        if session != Some(self.id.as_str()) {
            return Ok(None);
        }
        Ok(match event["type"].as_str().unwrap_or_default() {
            "session.idle" => Some(AgentEvent::Done(String::new())),
            "session.error" => {
                let error = &properties["error"];
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
            "permission.asked" => {
                // Anything else Jig can't show: say no, so the agent moves on.
                let id = properties["id"].as_str().unwrap_or_default();
                self.reply(id, false, Some("That isn't available inside Jig."))?;
                None
            }
            "message.part.updated" => {
                let part = &properties["part"];
                (part["type"] == "tool" && part["state"]["status"] == "running").then(|| {
                    AgentEvent::Step(describe(
                        part["tool"].as_str().unwrap_or_default(),
                        &part["state"]["input"],
                        &self.directory,
                    ))
                })
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
        let text = last
            .and_then(|message| message["parts"].as_array())
            .into_iter()
            .flatten()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str())
            .collect::<Vec<_>>()
            .join(" ");
        Ok(text.trim().to_string())
    }

    /// Let an edit through, or turn it down with an optional note the agent
    /// sees.
    pub fn reply(&self, edit_id: &str, accept: bool, note: Option<&str>) -> Result<()> {
        let mut body = json!({"reply": if accept { "once" } else { "reject" }});
        if let Some(note) = note {
            body["message"] = json!(note);
        }
        self.server
            .request(
                "POST",
                &format!("/permission/{edit_id}/reply"),
                &self.directory,
                Some(&body),
            )
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

/// A short, human-readable line for a tool call.
fn describe(tool: &str, input: &Value, directory: &str) -> String {
    let path = || {
        let path = input["filePath"]
            .as_str()
            .or_else(|| input["path"].as_str())
            .unwrap_or("");
        let relative = path
            .strip_prefix(directory)
            .map(|rest| rest.trim_start_matches('/'))
            .unwrap_or(path);
        if relative.is_empty() {
            ".".to_string()
        } else {
            relative.to_string()
        }
    };
    match tool {
        "read" => format!("Reading {}", path()),
        "list" => format!("Listing {}", path()),
        "edit" | "write" | "patch" | "multiedit" => format!("Editing {}", path()),
        "glob" | "grep" => format!(
            "Searching for {}",
            input["pattern"].as_str().unwrap_or("files")
        ),
        "skill" => format!("Using skill {}", input["name"].as_str().unwrap_or("")),
        "todowrite" | "todoread" => "Planning".into(),
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
