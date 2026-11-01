//! What OpenCode's event stream says, turned into [`AgentEvent`]s: the
//! conversation's own events and its subagents'.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use anyhow::{Result, bail};
use serde_json::Value;

use super::transcript::{CREATE_TASK, describe, did, permission, subagent_did};
use crate::domain::agent::{AgentEvent, EditChange, EditRequest, Question, QuestionRequest};
use crate::domain::map;

/// What an event comes to.
pub(crate) enum Heard {
    Event(AgentEvent),
    /// A request Jig can't show, by its id: turned down, so the agent
    /// moves on.
    Unavailable(String),
}

/// Turn one server event into something the UI cares about. Events of
/// the subagents it starts count too: their edits wait for the user
/// like its own.
pub(crate) fn interpret(
    event: &Value,
    sessions: &mut Sessions,
    own: &str,
    directory: &str,
) -> Result<Option<Heard>> {
    let properties = &event["properties"];
    let kind = event["type"].as_str().unwrap_or_default();
    if matches!(kind, "session.created" | "session.updated")
        && let (Some(id), Some(parent)) = (
            properties["info"]["id"].as_str(),
            properties["info"]["parentID"].as_str(),
        )
        && sessions.adopt(id, parent)
    {
        return Ok(Some(Heard::Event(AgentEvent::Subagent {
            id: id.to_string(),
            description: subagent_title(properties["info"]["title"].as_str().unwrap_or("")),
        })));
    }
    let session = properties["sessionID"]
        .as_str()
        .or_else(|| properties["part"]["sessionID"].as_str());
    if !session.is_some_and(|session| sessions.follows(session)) {
        return Ok(None);
    }
    let ours = session == Some(own);
    let subagent = session.filter(|_| !ours);
    let event = match kind {
        "session.idle" if ours => Some(AgentEvent::Done(String::new())),
        "session.error" if ours => {
            let error = &properties["error"];
            // Stopping the agent isn't a failure.
            if error["name"] == "MessageAbortedError" {
                return Ok(None);
            }
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
                change: EditChange::Diff(
                    metadata["diff"].as_str().unwrap_or_default().to_string(),
                ),
            }))
        }
        "permission.asked" => {
            let draft = sessions.draft_for(properties);
            return Ok(Some(match permission(properties, draft) {
                Some(request) => Heard::Event(AgentEvent::Permission(request)),
                // Anything else Jig can't show: say no, so the agent moves on.
                None => Heard::Unavailable(
                    properties["id"].as_str().unwrap_or_default().to_string(),
                ),
            }));
        }
        "question.asked" => Some(AgentEvent::Question(QuestionRequest {
            id: properties["id"].as_str().unwrap_or_default().to_string(),
            questions: properties["questions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|question| Question {
                    question: question["question"].as_str().unwrap_or_default().into(),
                    options: question["options"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(|option| option["label"].as_str().map(str::to_string))
                        .collect(),
                })
                .collect(),
        })),
        "message.part.updated" => {
            let part = &properties["part"];
            let tool = part["tool"].as_str().unwrap_or_default();
            let state = &part["state"];
            // The `task` tool starts a subagent; its child session tells
            // them apart, its output is what the subagent reported.
            let child = state["metadata"]["sessionId"].as_str();
            sessions.keep_draft(part);
            match (state["status"].as_str(), child) {
                (Some("running"), Some(child)) if tool == "task" => {
                    Some(AgentEvent::Subagent {
                        id: child.to_string(),
                        description: state["input"]["description"]
                            .as_str()
                            .unwrap_or("Subagent")
                            .to_string(),
                    })
                }
                (Some("completed" | "error"), Some(child)) if tool == "task" => {
                    Some(AgentEvent::SubagentDone {
                        id: child.to_string(),
                        report: task_report(state),
                    })
                }
                (Some("running"), _) if part["type"] == "tool" => Some(AgentEvent::Step(
                    describe(tool, &state["input"], directory),
                )),
                (Some("completed"), None) if map::tool_of(tool).is_some() => {
                    Some(AgentEvent::Drew {
                        tool: map::tool_of(tool).unwrap_or_default().to_string(),
                        input: state["input"].clone(),
                    })
                }
                (Some("completed"), _) if part["type"] == "tool" => match subagent {
                    Some(id) => Some(AgentEvent::SubagentDid {
                        id: id.to_string(),
                        text: subagent_did(tool, &state["input"], directory),
                    }),
                    None => did(tool, &state["input"]).map(AgentEvent::Did),
                },
                _ => None,
            }
        }
        _ => None,
    };
    Ok(event.map(Heard::Event))
}

/// The sessions a turn follows: the conversation's own, and its
/// subagents', each started in a child session (and theirs, as deep as
/// they go), each subagent known by its session.
pub(crate) struct Sessions {
    own: String,
    subagents: HashSet<String>,
    /// The plans `create_task` was called with, by call: its permission
    /// doesn't carry them, and the user is asked about the plan.
    drafts: HashMap<String, Value>,
}

impl Sessions {
    pub(crate) fn new(own: &str) -> Self {
        Self {
            own: own.to_string(),
            subagents: HashSet::new(),
            drafts: HashMap::new(),
        }
    }

    /// Follow `id` if it's a child of a session followed. True when it's
    /// newly a subagent.
    pub(super) fn adopt(&mut self, id: &str, parent: &str) -> bool {
        self.follows(parent) && id != self.own && self.subagents.insert(id.to_string())
    }

    pub(super) fn follows(&self, session: &str) -> bool {
        session == self.own || self.subagents.contains(session)
    }

    /// Remember the plan of a `create_task` call, once its input is known.
    pub(super) fn keep_draft(&mut self, part: &Value) {
        let input = &part["state"]["input"];
        if part["tool"] == CREATE_TASK
            && let Some(call) = part["callID"].as_str()
            && input.is_object()
        {
            self.drafts.insert(call.to_string(), input.clone());
        }
    }

    /// The plan of the call a permission is for.
    pub(super) fn draft_for(&mut self, properties: &Value) -> Option<Value> {
        self.drafts.remove(properties["tool"]["callID"].as_str()?)
    }
}

/// A child session's title without what OpenCode adds to it: "Find the
/// loader (@explore subagent)" is "Find the loader".
pub(super) fn subagent_title(title: &str) -> String {
    let title = match title.rfind(" (@") {
        Some(at) if title.ends_with(')') => &title[..at],
        _ => title,
    };
    match title.trim() {
        "" => "Subagent".into(),
        title => title.to_string(),
    }
}

/// What a subagent reported: the `task` tool's output, without the session
/// details OpenCode appends for the agent.
pub(super) fn task_report(state: &Value) -> String {
    let output = state["output"]
        .as_str()
        .or_else(|| state["error"].as_str())
        .unwrap_or_default();
    let output = output
        .find("<task_metadata>")
        .map_or(output, |at| &output[..at]);
    match output.trim() {
        "" => "Done.".into(),
        output => output.to_string(),
    }
}
