//! OpenCode's messages and tool calls in words: the transcript of an
//! earlier conversation, the lines for what the agent did, and the
//! requests the user is asked.

use serde_json::Value;

use crate::domain::agent::{
    Asking, PermissionRequest, TaskDraft, Turn, asked, relative,
};
use crate::domain::map;

/// The text a message says, paragraph by paragraph.
pub(crate) fn text_of(message: &Value) -> String {
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
pub(crate) fn history(messages: &Value, directory: &str) -> Vec<Turn> {
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
                    } else if completed && let Some(map_tool) = map::tool_of(tool) {
                        turns.push(Turn::Drew {
                            tool: map_tool.to_string(),
                            input: state["input"].clone(),
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
pub(super) fn permission(properties: &Value, input: Option<Value>) -> Option<PermissionRequest> {
    let metadata = &properties["metadata"];
    let pattern = properties["patterns"]
        .as_array()
        .and_then(|patterns| patterns.first())
        .and_then(Value::as_str)
        .unwrap_or_default();
    let field = |name: &str| metadata[name].as_str().map(str::to_string);
    let (title, detail, kind) = match properties["permission"].as_str()? {
        "bash" => (
            "Run a command",
            field("command").unwrap_or_else(|| pattern.into()),
            Asking::Command,
        ),
        "webfetch" => (
            "Fetch a page",
            field("url").unwrap_or_else(|| pattern.into()),
            Asking::Web,
        ),
        "websearch" => (
            "Search the web",
            field("query").unwrap_or_else(|| pattern.into()),
            Asking::Web,
        ),
        // Its pattern is the kind of subagent.
        "task" => (
            "Start a subagent",
            field("description").unwrap_or_else(|| pattern.into()),
            Asking::Subagent {
                explores: field("subagent_type").as_deref().unwrap_or(pattern) == "explore",
            },
        ),
        "doom_loop" => ("Repeat the same step again", pattern.into(), Asking::Other),
        CREATE_TASK => ("Save this plan as a task", String::new(), Asking::Other),
        "jig_update_task" => ("Update a task", String::new(), Asking::Other),
        "jig_list_tasks" => ("Read the tasks", String::new(), Asking::Other),
        _ => return None,
    };
    // OpenCode remembers these patterns once allowed always.
    let patterns: Vec<String> = properties["always"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|pattern| pattern.as_str())
        .filter(|pattern| *pattern != "*")
        .map(|pattern| format!("`{pattern}`"))
        .collect();
    Some(PermissionRequest {
        id: properties["id"].as_str()?.to_string(),
        title: title.into(),
        detail,
        kind,
        always: Some(if patterns.is_empty() {
            "Always allow".into()
        } else {
            format!("Always allow {}", patterns.join(", "))
        }),
        task: (properties["permission"] == CREATE_TASK)
            .then(|| TaskDraft::from_input(input.as_ref().unwrap_or(&Value::Null))),
    })
}

/// `create_task` as OpenCode names it, under Jig's tool server.
pub(super) const CREATE_TASK: &str = "jig_create_task";

fn is_edit(tool: &str) -> bool {
    matches!(
        tool,
        "edit" | "write" | "patch" | "multiedit" | "apply_patch"
    )
}

/// The transcript's line for a tool call worth keeping, once it's done.
/// Reading and searching are left out; edits have lines of their own.
pub(super) fn did(tool: &str, input: &Value) -> Option<String> {
    let field = |name: &str| input[name].as_str().unwrap_or_default().trim().to_string();
    Some(match tool {
        "bash" => format!("Ran `{}`", field("command")),
        "webfetch" => format!("Fetched {}", field("url")),
        "websearch" => format!("Searched the web for {}", field("query")),
        "task" => format!("Subagent: {}", field("description")),
        "skill" => format!("Used skill {}", field("name")),
        "jig_create_task" => format!("Saved the task {}", field("title")),
        _ => return None,
    })
}

/// A line for any finished call of a subagent's, reading and searching
/// included: it's the only way to see what it did.
pub(super) fn subagent_did(tool: &str, input: &Value, directory: &str) -> String {
    if let Some(line) = did(tool, input) {
        return line;
    }
    let path = || relative(&path_of(input), directory);
    match tool {
        "read" => format!("Read {}", path()),
        "list" => format!("Listed {}", path()),
        tool if is_edit(tool) => format!("Edited {}", path()),
        "glob" | "grep" => format!(
            "Searched for {}",
            input["pattern"].as_str().unwrap_or("files")
        ),
        other => format!("Used {other}"),
    }
}

fn path_of(input: &Value) -> String {
    input["filePath"]
        .as_str()
        .or_else(|| input["path"].as_str())
        .unwrap_or("")
        .to_string()
}

/// A short, human-readable line for a tool call.
pub(super) fn describe(tool: &str, input: &Value, directory: &str) -> String {
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
        "jig_create_task" => "Saving a task".into(),
        "jig_list_tasks" => "Reading the tasks".into(),
        "jig_update_task" => "Updating a task".into(),
        other => format!("Running {other}"),
    }
}
