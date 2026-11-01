//! What an ACP agent asks of Jig mid-turn: a go-ahead for a tool call or
//! an edit, or a file to write. Answered at once when the settings say so,
//! otherwise put to the user.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use anyhow::Result;
use serde_json::{Value, json};

use super::{Session, Waiting};
use crate::domain::acp::{TurnState, outcome, pick};
use crate::domain::agent::{
    Access, AgentEvent, EditChange, EditRequest, PermissionRequest, TaskDraft,
};
use crate::store::acp::lock;

impl Session {
    /// A request from the agent: answered at once when the settings say so,
    /// otherwise put to the user as an event.
    pub(super) fn ask(
        &self,
        turn: &mut TurnState,
        id: Value,
        method: &str,
        params: &Value,
    ) -> Result<Option<AgentEvent>> {
        let ours = format!("acp-{}", self.next.fetch_add(1, Ordering::Relaxed));
        match method {
            "session/request_permission" => {
                let call = turn.merge(&params["toolCall"]);
                let options = params["options"].clone();
                let answer = |kinds: &[&str]| {
                    let option = pick(&options, kinds);
                    self.connection.respond(&id, Ok(outcome(option.as_deref())))
                };
                if let Some(edit) = call.edit() {
                    turn.reviewed(&call.id);
                    let waiting = Waiting::Permission {
                        id,
                        options,
                        edit: Some(edit.path.clone()),
                    };
                    lock(&self.waiting).insert(ours.clone(), waiting);
                    return Ok(Some(AgentEvent::Edit(EditRequest { id: ours, ..edit })));
                }
                let access = if call.map_tool().is_some() {
                    // Drawing on the Map changes no code.
                    Access::Allow
                } else if call.creates_task() {
                    turn.permissions.creating_tasks()
                } else if call.is_task_tool() {
                    turn.permissions.tasks
                } else {
                    turn.permissions.access(call.kind.as_str())
                };
                match access {
                    Access::Allow => answer(&["allow_once", "allow_always"])?,
                    Access::Deny => answer(&["reject_once", "reject_always"])?,
                    Access::Ask => {
                        // The agent's own "don't ask again", if it has one.
                        let always = options.as_array().and_then(|options| {
                            let option = options
                                .iter()
                                .find(|option| option["kind"] == "allow_always")?;
                            Some(
                                option["name"]
                                    .as_str()
                                    .unwrap_or("Always allow")
                                    .to_string(),
                            )
                        });
                        let task = call
                            .creates_task()
                            .then(|| TaskDraft::from_input(&call.raw_input));
                        let request = PermissionRequest {
                            id: ours.clone(),
                            title: match task {
                                Some(_) => "Save this plan as a task".into(),
                                None => call.asking(),
                            },
                            detail: call.detail(&self.directory),
                            kind: call.what(),
                            always,
                            task,
                        };
                        let waiting = Waiting::Permission {
                            id,
                            options,
                            edit: None,
                        };
                        lock(&self.waiting).insert(ours, waiting);
                        return Ok(Some(AgentEvent::Permission(request)));
                    }
                }
                Ok(None)
            }
            "fs/write_text_file" => {
                let path = PathBuf::from(params["path"].as_str().unwrap_or_default());
                let content = params["content"].as_str().unwrap_or_default().to_string();
                turn.wrote(&path);
                // An edit the user just allowed, which the agent leaves to Jig
                // to write: no need to ask twice.
                let allowed = {
                    let mut allowed = lock(&self.allowed);
                    let found = allowed.iter().position(|file| *file == path);
                    found.map(|at| allowed.remove(at)).is_some()
                };
                if allowed {
                    let written = std::fs::write(&path, &content)
                        .map(|()| json!({}))
                        .map_err(|error| format!("{}: {error}", path.display()));
                    self.connection.respond(&id, written)?;
                    return Ok(None);
                }
                lock(&self.waiting).insert(
                    ours.clone(),
                    Waiting::Write {
                        id,
                        path: path.clone(),
                    },
                );
                Ok(Some(AgentEvent::Edit(EditRequest {
                    id: ours,
                    path,
                    change: EditChange::Write(content),
                })))
            }
            _ => {
                self.connection
                    .respond(&id, Err("That isn't available inside Jig.".into()))?;
                Ok(None)
            }
        }
    }
}
