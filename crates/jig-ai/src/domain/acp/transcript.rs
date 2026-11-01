//! An earlier ACP conversation, as the agent replays it on `session/load`.

use std::collections::HashMap;

use serde_json::Value;

use super::ToolCall;
use crate::domain::agent::{Turn, asked};

/// An earlier conversation as the agent replays it.
pub(crate) struct Transcript {
    pub(crate) directory: String,
    pub(crate) turns: Vec<Turn>,
    pub(crate) calls: HashMap<String, ToolCall>,
    /// The message being replayed, by who said it and its id.
    pub(crate) current: Option<(bool, Option<String>)>,
}

impl Transcript {
    pub(crate) fn new(directory: &str) -> Self {
        Self {
            directory: directory.to_string(),
            turns: Vec::new(),
            calls: HashMap::new(),
            current: None,
        }
    }

    pub(crate) fn add(&mut self, update: &Value) {
        use Turn as Line;
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
    pub(crate) fn finish(self) -> Vec<Turn> {
        use Turn as Line;
        self.turns
            .into_iter()
            .filter_map(|line| match line {
                Line::User(text) => Some(Line::User(asked(&text).to_string())),
                Line::Agent(text) if text.trim().is_empty() => None,
                Line::Agent(text) => Some(Line::Agent(text.trim().to_string())),
                other => Some(other),
            })
            .collect()
    }
}
