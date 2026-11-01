//! Conversations with a coding agent, the way Jig's AI works.

mod diff;
mod prompt;

use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::{Result, bail};
use serde_json::Value;

pub use diff::apply_diff;
pub(crate) use diff::relative;
pub use prompt::{CodeReference, Diagnostic, Prompt, asked, message, prompt};

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

    /// As `settings.toml` writes it.
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
    /// Updating tasks in Jig's task planner, which only writes
    /// `.jig/tasks/`: allowed unless the user says otherwise. Saving a new
    /// one is always put to the user as a question, unless this denies it
    /// ([`Permissions::creating_tasks`]).
    pub tasks: Access,
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
            tasks: Access::Allow,
            questions: true,
        }
    }
}

impl Permissions {
    /// Saving a plan as a new task: the agent proposes it with
    /// `create_task` and the user is asked whether to save it, the
    /// question being the call's go-ahead. Only turning tasks off skips it.
    pub fn creating_tasks(&self) -> Access {
        match self.tasks {
            Access::Deny => Access::Deny,
            Access::Ask | Access::Allow => Access::Ask,
        }
    }
}

/// The longest title Jig gives a conversation, in characters.
const TITLE_CHARS: usize = 60;

/// An earlier conversation with the agent, to pick up again.
#[derive(Clone, Debug, PartialEq)]
pub struct Conversation {
    pub id: String,
    /// What the user first asked, or `None` for conversations from before
    /// Jig named them.
    pub title: Option<String>,
    pub updated: SystemTime,
}

/// A command or skill the agent runs by name, as in "/review".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCommand {
    pub name: String,
    /// Its first line.
    pub description: String,
    pub skill: bool,
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

/// What the agent is asked to do.
#[derive(Clone, Debug)]
pub struct AgentRequest {
    /// The project folder the agent works in.
    pub directory: PathBuf,
    /// The message, or a command's arguments.
    pub prompt: String,
    /// The model, as the agent names it: `provider/model` for OpenCode.
    pub model: String,
    /// Run this command or skill, e.g. "review", with `prompt` as its
    /// arguments.
    pub command: Option<String>,
    /// What it may do this turn.
    pub permissions: Permissions,
}

/// One edit the agent wants to make, waiting for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditRequest {
    pub id: String,
    pub path: PathBuf,
    pub change: EditChange,
}

/// How an agent says what an edit changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditChange {
    /// A unified diff against the file on disk. The agent writes it once
    /// it's accepted.
    Diff(String),
    /// `old` replaced by `new`, each the whole file or a part of it; `old`
    /// is `None` for a new file. The agent writes it once it's accepted.
    Replace { old: Option<String>, new: String },
    /// The file's whole new text. Jig writes it once it's accepted.
    Write(String),
}

impl EditChange {
    /// The file's text after the edit, from its text on disk.
    pub fn apply(&self, disk: &str) -> Result<String> {
        match self {
            EditChange::Diff(diff) => apply_diff(disk, diff),
            EditChange::Write(text) => Ok(text.clone()),
            EditChange::Replace { old, new } => {
                match old.as_deref().filter(|old| !old.is_empty()) {
                    None => Ok(new.clone()),
                    Some(old) if old == disk => Ok(new.clone()),
                    Some(old) => match disk.matches(old).count() {
                        1 => Ok(disk.replacen(old, new, 1)),
                        0 => bail!("the file changed since the agent read it"),
                        _ => bail!("the text to change is in the file more than once"),
                    },
                }
            }
        }
    }

    /// Jig, not the agent, puts the accepted text on disk.
    pub fn written_by_jig(&self) -> bool {
        matches!(self, EditChange::Write(_))
    }
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
    /// What it does, whatever the agent titles it.
    pub kind: Asking,
    /// What allowing it always would cover, as the agent remembers it,
    /// e.g. "Always allow `cargo test *`". `None` when the agent can't
    /// remember; Jig then stops asking for the rest of the conversation.
    pub always: Option<String>,
    /// The plan it wants saved as a task, when that's what it asks: put to
    /// the user as "Save this plan as a task?" rather than as a go-ahead.
    pub task: Option<TaskDraft>,
}

/// A plan the agent proposes with `create_task`, as the user is asked it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TaskDraft {
    pub title: String,
    pub todos: Vec<String>,
}

impl TaskDraft {
    /// From `create_task`'s input; what's missing is left empty.
    pub fn from_input(input: &Value) -> Self {
        Self {
            title: input["title"].as_str().unwrap_or_default().trim().into(),
            todos: input["todos"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|todo| !todo.is_empty())
                .map(str::to_string)
                .collect(),
        }
    }
}

/// What a request does, so it's told apart by that rather than by how an
/// agent words it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Asking {
    /// A shell command, the request's detail.
    Command,
    /// Reading or searching the project.
    Read,
    /// Fetching a page or searching the web.
    Web,
    /// Starting a subagent; `explores` when it's one that only looks around
    /// (OpenCode's `explore`, Claude Code's `Explore`).
    Subagent { explores: bool },
    #[default]
    Other,
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
    /// A subagent it started, known by the session it runs in, and what it
    /// was given to do. Sent again as more is known; the last one counts.
    Subagent {
        id: String,
        description: String,
    },
    /// A line of a subagent's own work, like [`AgentEvent::Did`].
    SubagentDid {
        id: String,
        text: String,
    },
    /// A subagent finished, with what it said last: its report back.
    SubagentDone {
        id: String,
        report: String,
    },
    /// The commands and skills it offers, when it says.
    Commands(Vec<AgentCommand>),
    /// It drew on the Map: which of the Map's tools (`map_trace`), with
    /// what it was given. Sent once the call is done; the app checks the
    /// places named, as the tool did.
    Drew {
        tool: String,
        input: Value,
    },
    /// The agent finished; its closing words.
    Done(String),
}

/// One line of an earlier conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Turn {
    User(String),
    Agent(String),
    Edit {
        path: String,
        accepted: bool,
    },
    Did(String),
    /// A drawing on the Map, as [`AgentEvent::Drew`].
    Drew {
        tool: String,
        input: Value,
    },
}

#[cfg(test)]
mod tests;
