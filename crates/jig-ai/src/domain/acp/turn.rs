//! What one turn of an ACP agent has seen: its tool calls, its words and
//! its subagents', turned into [`AgentEvent`]s as they come.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{ToolCall, commands};
use crate::domain::agent::{AgentEvent, Permissions};

/// What a turn has seen so far.
pub(crate) struct TurnState {
    pub(crate) directory: String,
    pub(crate) permissions: Permissions,
    pub(crate) calls: HashMap<String, ToolCall>,
    /// Tool calls and files the user reviewed, so they aren't listed as
    /// changed without asking.
    pub(crate) reviewed: Vec<String>,
    pub(crate) written: Vec<PathBuf>,
    /// The agent's words, message by message.
    pub(crate) messages: Vec<(Option<String>, String)>,
    /// Each subagent's words, by its session: the last is its report.
    pub(crate) subagents: HashMap<String, Vec<(Option<String>, String)>>,
}

impl TurnState {
    pub(crate) fn new(directory: &str, permissions: Permissions) -> Self {
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

    pub(crate) fn merge(&mut self, update: &Value) -> ToolCall {
        let id = update["toolCallId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let call = self.calls.entry(id).or_default();
        call.merge(update);
        call.clone()
    }

    pub(crate) fn reviewed(&mut self, call: &str) {
        self.reviewed.push(call.to_string());
    }

    pub(crate) fn wrote(&mut self, path: &Path) {
        self.written.push(path.to_path_buf());
    }

    pub(crate) fn update(&mut self, update: &Value) -> Vec<AgentEvent> {
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
    pub(crate) fn subagent_update(&mut self, session: &str, update: &Value) -> Vec<AgentEvent> {
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
    pub(crate) fn tool_call(&mut self, update: &Value, subagent: Option<&str>) -> Vec<AgentEvent> {
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

    pub(crate) fn reply(&self) -> String {
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
