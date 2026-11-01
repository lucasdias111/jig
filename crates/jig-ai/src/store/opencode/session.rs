//! One conversation on the OpenCode server: starting or picking it up,
//! running a turn while reading the server's events, and answering what
//! the agent asks.

use std::io::{BufRead as _, BufReader};
use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::{Value, json};

use super::Server;
use crate::domain::agent::{AgentEvent, AgentRequest, Permissions, Turn, title as title_of};
use crate::domain::opencode::{self, Heard, SYSTEM, Sessions, rules};
use crate::store::canonical;

/// One conversation with the agent. Cheap to clone, so the UI can reply to
/// edits and abort while another thread runs [`Session::run`].
#[derive(Clone)]
pub struct Session {
    server: Arc<Server>,
    id: String,
    directory: String,
}

impl Session {
    /// A new conversation, titled with what the user asked.
    pub fn create(
        server: Arc<Server>,
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
                "title": title_of(title),
                "permission": rules(permissions),
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
    pub fn open(server: Arc<Server>, directory: &Path, id: &str) -> Result<Self> {
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
        Ok(opencode::history(&messages, &self.directory))
    }

    /// Send `request` and report what happens until the agent is done.
    /// Blocks; edits and other requests wait until [`Session::reply`]
    /// or [`Session::answer`] is called.
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
            Some(&json!({"permission": rules(&request.permissions)})),
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
                    let directory = &session.directory;
                    if let Err(error) =
                        session
                            .server
                            .send("POST", &path, directory, &[], Some(&body), None)
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

        let mut sessions = Sessions::new(&self.id);
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
            match self.interpret(&event, &mut sessions)? {
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

    /// Turn one server event into something the UI cares about, turning
    /// down what Jig can't show so the agent moves on.
    fn interpret(&self, event: &Value, sessions: &mut Sessions) -> Result<Option<AgentEvent>> {
        Ok(match opencode::interpret(event, sessions, &self.id, &self.directory)? {
            Some(Heard::Event(event)) => Some(event),
            Some(Heard::Unavailable(id)) => {
                self.reply(&id, false, Some("That isn't available inside Jig."))?;
                None
            }
            None => None,
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
        Ok(last.map(opencode::text_of).unwrap_or_default())
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

    /// Let a request through, and others like it from now on.
    pub fn allow_always(&self, request_id: &str) -> Result<()> {
        self.server
            .request(
                "POST",
                &format!("/permission/{request_id}/reply"),
                &self.directory,
                Some(&json!({"reply": "always"})),
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

    /// Stop the agent. [`Session::run`] then returns.
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
