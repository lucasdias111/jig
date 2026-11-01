//! The Agent Client Protocol as plain data, for Jig's client in the store: any agent that speaks ACP
//! (Claude Code and Codex through their adapters, Gemini CLI, OpenCode,
//! Goose, …) run as a child process and spoken to in JSON-RPC over stdio.
//! Here is what its messages say and what Jig answers; the process and
//! the conversation's transport are in `store/acp`.
//!
//! Jig keeps the last word here too, as far as the agent lets it: an edit it
//! asks permission for, or hands Jig to write (`fs/write_text_file`), is
//! shown as a normal preview first. Other requests (shell commands, the
//! web) follow [`Permissions`]. An agent set to edit without asking can't
//! be stopped; Jig lists what it changed.

mod tool_call;
mod transcript;
mod turn;

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use super::agent::{Access, AgentCommand, Conversation, Permissions};

pub(crate) use tool_call::ToolCall;
pub(crate) use transcript::Transcript;
pub(crate) use turn::TurnState;

/// How to start an agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCommandLine {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// `PATH` for the agent, which may start other programs.
    pub path: Option<std::ffi::OsString>,
}

/// ACP's code for "sign in first".
pub(crate) const AUTH_REQUIRED: i64 = -32000;

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

/// The ways to sign in an agent started with `command` offers, from its
/// `authMethods`: a terminal one runs `command` with the method's args.
pub(crate) fn sign_ins(methods: &[Value], command: &AgentCommandLine) -> Vec<SignIn> {
    methods
        .iter()
        .filter_map(|method| {
            let terminal = (method["type"] == "terminal").then(|| AgentCommandLine {
                args: command
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
                ..command.clone()
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

/// The conversations `session/list` answered with, latest first.
pub(crate) fn conversations(listed: &Value) -> Vec<Conversation> {
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
    found
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
    pub(crate) fn from_options(options: &Value) -> Option<Self> {
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

impl Permissions {
    /// What the settings say about a tool call of ACP kind `kind`.
    pub(crate) fn access(&self, kind: &str) -> Access {
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
pub(crate) fn pick(options: &Value, kinds: &[&str]) -> Option<String> {
    kinds.iter().find_map(|kind| {
        options
            .as_array()?
            .iter()
            .find(|option| option["kind"] == *kind)?["optionId"]
            .as_str()
            .map(str::to_string)
    })
}

pub(crate) fn outcome(option: Option<&str>) -> Value {
    match option {
        Some(option) => json!({"outcome": {"outcome": "selected", "optionId": option}}),
        None => json!({"outcome": {"outcome": "cancelled"}}),
    }
}

/// The commands an agent announces in `available_commands_update`.
pub(crate) fn commands(list: &Value) -> Vec<AgentCommand> {
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

/// An ISO 8601 time such as "2026-10-10T05:38:50.337Z", in UTC.
pub(crate) fn parse_time(text: &str) -> Option<SystemTime> {
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

#[cfg(test)]
mod tests;
