//! OpenCode's side of Jig's client, as plain data: the config and rules it
//! is started and run with, and what its answers and events say.
//!
//! Edits are set to "ask", so each one arrives as a diff before anything is
//! written, and OpenCode writes it once the user accepts. Conversations stay
//! in OpenCode, which Jig marks as its own so they can be listed again.

mod events;
mod transcript;

use std::path::Path;
use std::time::{Duration, SystemTime};

use serde_json::{Value, json};

use super::agent::{Access, AgentCommand, Conversation, Permissions};
use super::config::{ProviderConfig, ProviderKind};
use super::{map, tasks};

pub(crate) use events::{Heard, Sessions, interpret};
pub(crate) use transcript::{history, text_of};

/// A model OpenCode knows by itself, for trying the agent on its own.
pub const DEFAULT_MODEL: &str = "opencode-go/qwen3.8-flash";

/// Room OpenCode assumes for a model Jig hands it; it can't look these up
/// for a model it doesn't know.
const CONTEXT_TOKENS: u32 = 128_000;
const OUTPUT_TOKENS: u32 = 16_000;

/// What an agent run needs from OpenCode: the model, as OpenCode names it,
/// and the provider config to start the server with.
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub model: String,
    /// Passed as `OPENCODE_CONFIG_CONTENT`. `None` for a model OpenCode
    /// knows by itself.
    pub config: Option<Value>,
}

/// One of Jig's providers as OpenCode's config takes it, its requests
/// marked with `session` when the provider asks for that.
pub(crate) fn opencode_provider(
    provider: &ProviderConfig,
    model: &str,
    api_key: Option<&str>,
    session: &str,
) -> Value {
    let npm = match provider.kind {
        ProviderKind::Anthropic => "@ai-sdk/anthropic",
        ProviderKind::Openai => "@ai-sdk/openai-compatible",
    };
    let mut options = json!({ "baseURL": provider.base_url.trim_end_matches('/') });
    if let Some(key) = api_key {
        options["apiKey"] = json!(key);
    }
    if let Some(header) = &provider.session_header {
        options["headers"] = json!({ header: session });
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
pub(crate) const SYSTEM: &str = "You are running inside Jig, a code editor. The user talks to you from the code itself; a message may say which file they are in and what they selected. Make changes with your edit tools. Each edit is shown to the user, who accepts or rejects it; a rejection may come with a note, which you should follow. Shell commands, web access and subagents may wait for the user's go-ahead or be turned off; if one is turned down, carry on without it or ask. Change only what the task needs. When you are done, say briefly what you did; Markdown is fine. If the task is unclear, ask with your question tool or in your reply; later messages continue the same conversation. Write a plan as a numbered list, one step per item. Whenever you propose three or more steps, or work that spans several files, propose it with the jig_create_task tool, short and laid out as the tool describes, rather than asking in your reply or with your question tool: Jig asks the user whether to save it, and they may answer with something to change instead.";

/// `permissions` as OpenCode takes them for a session.
pub(crate) fn rules(permissions: &Permissions) -> Value {
    let questions = if permissions.questions {
        Access::Allow
    } else {
        Access::Deny
    };
    [
            ("edit", Access::Ask),
            ("bash", permissions.shell),
            ("task", permissions.subagents),
            ("webfetch", permissions.web_fetch),
            ("websearch", permissions.web_search),
            ("question", questions),
            ("external_directory", Access::Deny),
        ]
        .iter()
        .map(|(permission, access)| (permission.to_string(), *access))
        .chain(
            tasks::mcp::TOOLS
                .iter()
                .map(|tool| {
                    let access = if *tool == tasks::mcp::CREATE {
                        permissions.creating_tasks()
                    } else {
                        permissions.tasks
                    };
                    (format!("{}_{tool}", tasks::mcp::NAME), access)
                }),
        )
        // Drawing on the Map changes no code: never asked about.
        .chain(map::opencode_permissions().map(|tool| (tool, Access::Allow)))
        .map(|(permission, access)| {
            json!({"permission": permission, "pattern": "*", "action": access.key()})
        })
        .collect()
}

/// `config` with Jig's task tool and the Map's tools added, started with
/// `command`, which OpenCode starts in each project's folder.
pub(crate) fn with_task_tool(config: Option<&Value>, command: Option<&Path>) -> Option<Value> {
    let Some(command) = command else {
        return config.cloned();
    };
    let mut config = config.cloned().unwrap_or_else(|| json!({}));
    config["mcp"][tasks::mcp::NAME] = tasks::mcp::opencode_server(command);
    config["mcp"][map::NAME] = map::opencode_server(command);
    Some(config)
}

/// The sessions Jig started, which are the ones whose edits ask first:
/// the user's own OpenCode sessions don't, so they aren't safe to continue
/// here unchanged.
pub(crate) fn conversations(sessions: &Value) -> Vec<Conversation> {
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

/// The commands and skills OpenCode lists at `GET /command`.
pub(crate) fn commands(list: &Value) -> Vec<AgentCommand> {
    list.as_array()
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
        .collect()
}

/// The `Authorization: Basic` encoding of `bytes`.
pub(crate) fn base64(bytes: &[u8]) -> String {
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

#[cfg(test)]
mod tests;
