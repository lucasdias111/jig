//! A tool call as an ACP agent describes it, and what Jig makes of it: an
//! edit, a request, a step, a line in the transcript.

use std::path::PathBuf;

use serde_json::Value;

use crate::domain::agent::{Asking, EditChange, EditRequest, relative};
use crate::domain::{map, tasks};

/// A tool call as it has been described so far: `tool_call` and each
/// `tool_call_update` fill it in.
#[derive(Clone, Debug, Default)]
pub(crate) struct ToolCall {
    pub(crate) id: String,
    pub(crate) title: String,
    /// The tool's own name, where the agent says it.
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) status: String,
    pub(crate) raw_input: Value,
    pub(crate) content: Vec<Value>,
    pub(crate) locations: Vec<String>,
}

impl ToolCall {
    pub(crate) fn merge(&mut self, update: &Value) {
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
    pub(crate) fn edit(&self) -> Option<EditRequest> {
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
    pub(crate) fn map_tool(&self) -> Option<&'static str> {
        map::tool_of(&self.name).or_else(|| map::tool_of(&self.title))
    }

    /// One of Jig's own task tools, as the agent names it: Claude Code's
    /// `mcp__jig__create_task`, Gemini's `create_task (jig MCP Server)`.
    pub(crate) fn is_task_tool(&self) -> bool {
        let title = self.title.to_lowercase();
        title.contains(tasks::mcp::NAME)
            && tasks::mcp::TOOLS.iter().any(|tool| title.contains(tool))
    }

    /// Jig's `create_task`, whose go-ahead is the user saving the plan.
    pub(crate) fn creates_task(&self) -> bool {
        self.is_task_tool() && self.title.to_lowercase().contains(tasks::mcp::CREATE)
    }

    pub(crate) fn command(&self) -> Option<String> {
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
    pub(crate) fn asking(&self) -> String {
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
    pub(crate) fn what(&self) -> Asking {
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

    pub(crate) fn detail(&self, directory: &str) -> String {
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
    pub(crate) fn step(&self, directory: &str) -> String {
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
    pub(crate) fn done(&self, directory: &str) -> Option<String> {
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
    pub(crate) fn summary(&self, directory: &str) -> String {
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

    pub(crate) fn edited_path(&self, directory: &str) -> Option<String> {
        self.content
            .iter()
            .find(|content| content["type"] == "diff")
            .and_then(|diff| diff["path"].as_str())
            .or_else(|| self.locations.first().map(String::as_str))
            .map(|path| relative(path, directory))
    }
}
