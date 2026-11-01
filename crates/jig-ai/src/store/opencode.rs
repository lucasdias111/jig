//! OpenCode, through Jig's own client: a private `opencode serve` spoken to
//! over HTTP, with its events read from `/event`.
//!
//! Edits are set to "ask", so each one arrives as a diff before anything is
//! written, and OpenCode writes it once the user accepts. Conversations stay
//! in OpenCode, which Jig marks as its own so they can be listed again.

mod session;

use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, mpsc};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use jig_process::NoWindow as _;
use serde_json::{Value, json};

use super::canonical;
use super::session::session_id;
use crate::domain::agent::{AgentCommand, Conversation};
use crate::domain::config::AgentModel;
use crate::domain::opencode::{self, Target};

pub use session::Session;

/// How long `opencode serve` gets to say where it listens. A first start
/// may install its packages, so it's generous; past it, something's stuck.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);
/// How long one ordinary request gets, all of it. The event stream and a
/// command's request, which only returns once the command is done, have none.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

impl Target {
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
            config: Some(json!({ "provider": { id: opencode::opencode_provider(
                    provider,
                    model,
                    api_key,
                    session_id(),
                ) } })),
        }
    }
}

/// A running `opencode serve`, private to this Jig process. It is stopped
/// when dropped.
pub struct Server {
    url: String,
    /// `Authorization` header value; the server only answers to Jig.
    auth: String,
    http: ureq::Agent,
    child: Mutex<Child>,
}

impl Server {
    /// Start the server with `config` added to the user's own OpenCode
    /// config, and wait until it listens.
    pub fn start(config: Option<&Value>) -> Result<Self> {
        let password = random_hex();
        let mut command = Command::new(opencode_binary());
        let tool = super::session::command().map(PathBuf::as_path);
        if let Some(config) = opencode::with_task_tool(config, tool) {
            command.env("OPENCODE_CONFIG_CONTENT", config.to_string());
        }
        let mut child = command
            .no_window()
            .args(["serve", "--port", "0", "--hostname", "127.0.0.1"])
            .env("OPENCODE_SERVER_PASSWORD", &password)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Couldn't start OpenCode. Is `opencode` installed?")?;
        let stdout = child.stdout.take().context("no stdout from opencode")?;
        // Read on a thread of its own, so a server that never says where it
        // listens can be given up on; it keeps reading after, so the server
        // never writes into a closed pipe.
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut tx = Some(tx);
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if let Some(rest) = line.split("listening on ").nth(1)
                    && let Some(tx) = tx.take()
                {
                    let _ = tx.send(rest.trim().trim_end_matches('/').to_string());
                }
            }
        });
        let url = match rx.recv_timeout(STARTUP_TIMEOUT) {
            Ok(url) => url,
            Err(why) => {
                let _ = child.kill();
                let _ = child.wait();
                match why {
                    mpsc::RecvTimeoutError::Timeout => bail!(
                        "OpenCode didn't start listening within {} s",
                        STARTUP_TIMEOUT.as_secs()
                    ),
                    mpsc::RecvTimeoutError::Disconnected => {
                        bail!("OpenCode exited before it started listening")
                    }
                }
            }
        };
        Ok(Self {
            url,
            auth: format!(
                "Basic {}",
                opencode::base64(format!("opencode:{password}").as_bytes())
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

    /// [`Server::request`] with more query parameters.
    fn request_with(
        &self,
        method: &str,
        path: &str,
        directory: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
    ) -> Result<Value> {
        self.send(method, path, directory, query, body, Some(REQUEST_TIMEOUT))
    }

    /// One request, given up on after `timeout`, or never with `None`.
    fn send(
        &self,
        method: &str,
        path: &str,
        directory: &str,
        query: &[(&str, &str)],
        body: Option<&Value>,
        timeout: Option<Duration>,
    ) -> Result<Value> {
        let url = format!("{}{path}", self.url);
        let query = std::iter::once(("directory", directory)).chain(query.iter().copied());
        let mut response = match (method, body) {
            ("PATCH", Some(body)) => self
                .http
                .patch(&url)
                .config()
                .timeout_global(timeout)
                .build()
                .query_pairs(query)
                .header("Authorization", &self.auth)
                .send_json(body),
            (_, Some(body)) => self
                .http
                .post(&url)
                .config()
                .timeout_global(timeout)
                .build()
                .query_pairs(query)
                .header("Authorization", &self.auth)
                .send_json(body),
            ("GET", None) => self
                .http
                .get(&url)
                .config()
                .timeout_global(timeout)
                .build()
                .query_pairs(query)
                .header("Authorization", &self.auth)
                .call(),
            (_, None) => self
                .http
                .post(&url)
                .config()
                .timeout_global(timeout)
                .build()
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
        Ok(opencode::conversations(&sessions))
    }

    /// The commands and skills OpenCode offers in `directory`.
    pub fn commands(&self, directory: &Path) -> Result<Vec<AgentCommand>> {
        let commands = self.request("GET", "/command", &canonical(directory), None)?;
        Ok(opencode::commands(&commands))
    }
}

impl Server {
    /// Stop the server process. Sessions still running on it fail.
    pub fn stop(&self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop();
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
