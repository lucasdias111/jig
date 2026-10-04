//! Agent commands: the command goes to a coding agent (OpenCode), which may
//! read and edit the whole project. Every edit it wants to make opens in the
//! editor as a normal preview, in its file's tab, and only goes through when
//! the user accepts it.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt as _;
use gpui_kit::*;
use jig_ai::PromptRequest;
use jig_ai::agent::{AgentEvent, AgentRequest, AgentSession, EditRequest};
use jig_commands::{Bubble, LiveStep};
use jig_editor::EditorHandle;

use super::Workspace;
use super::commands::Preview;
use super::tabs::canonical;

/// What the agent's thread reports to the window.
enum Message {
    Started(AgentSession),
    Event(AgentEvent),
    Failed(String),
}

/// The agent side of a [`CommandRun`].
pub(super) struct AgentRun {
    /// `None` until the session is set up.
    session: Option<AgentSession>,
    /// The edit on screen, awaiting accept or reject.
    pending: Option<PendingEdit>,
    /// Edits that came in while another was on screen.
    queue: VecDeque<EditRequest>,
    /// Files the agent changed, to catch up with the disk when it's done
    /// (OpenCode may format a file after writing it).
    touched: Vec<PathBuf>,
    step: LiveStep,
    label: String,
    started: std::time::Instant,
    context: Vec<String>,
    finished: bool,
}

struct PendingEdit {
    id: String,
    path: PathBuf,
    /// The file didn't exist; it is removed again if the edit is rejected.
    created: bool,
}

impl Drop for AgentRun {
    /// Cancelled (Esc, another command, switching tabs): stop the agent.
    fn drop(&mut self) {
        if !self.finished
            && let Some(session) = self.session.clone()
        {
            std::thread::spawn(move || session.abort());
        }
    }
}

impl Workspace {
    pub(super) fn run_agent(
        &mut self,
        id: u64,
        label: String,
        request: PromptRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = self.document().path.clone() else {
            return self.fail(
                id,
                "Save the file first, so the agent knows which project it's in.".into(),
                window,
                cx,
            );
        };
        // The agent reads the disk, so it must see what's on screen.
        if self.tab().dirty {
            self.save_to(&path, window, cx);
        }
        let directory = crate::project::root_for(&path);
        let prompt = jig_ai::agent::prompt(&request);
        let step = LiveStep::default();
        step.set("Starting the agent".into());

        let (sender, mut messages) = futures::channel::mpsc::unbounded();
        std::thread::spawn(move || {
            let send = |message| {
                let _ = sender.unbounded_send(message);
            };
            let result = (|| {
                let server = crate::agent::server()?;
                let session = AgentSession::create(server, &directory)?;
                send(Message::Started(session.clone()));
                let request = AgentRequest {
                    directory,
                    prompt,
                    model: crate::agent::model(),
                };
                session.run(&request, &|event| send(Message::Event(event)))
            })();
            if let Err(error) = result {
                send(Message::Failed(format!("{error:#}")));
            }
        });
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Some(message) = messages.next().await {
                let alive = this
                    .update_in(cx, |this, window, cx| {
                        this.on_agent_message(id, message, window, cx)
                    })
                    .unwrap_or(false);
                if !alive {
                    break;
                }
            }
        });

        let started = std::time::Instant::now();
        let context = self.context_names();
        let run = self.run.as_mut().expect("run_command set up the run");
        run.bubble = Bubble::Running {
            label: label.clone(),
            agent: true,
            started,
            context: context.clone(),
            step: Some(step.clone()),
        };
        run.agent = Some(AgentRun {
            session: None,
            pending: None,
            queue: VecDeque::new(),
            touched: Vec::new(),
            step,
            label,
            started,
            context,
            finished: false,
        });
        run._task = Some(task);
        cx.notify();
    }

    fn agent_run(&mut self, id: u64) -> Option<&mut AgentRun> {
        self.run
            .as_mut()
            .filter(|run| run.id == id)
            .and_then(|run| run.agent.as_mut())
    }

    /// Handle one message from the agent's thread. Returns false once the
    /// run is over or gone.
    fn on_agent_message(
        &mut self,
        id: u64,
        message: Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(agent) = self.agent_run(id) else {
            return false;
        };
        match message {
            Message::Started(session) => agent.session = Some(session),
            Message::Event(AgentEvent::Step(step)) => agent.step.set(step),
            Message::Event(AgentEvent::Edit(edit)) => agent.queue.push_back(edit),
            Message::Event(AgentEvent::Done(text)) => {
                agent.finished = true;
                self.finish_agent(text, window, cx);
                return false;
            }
            Message::Failed(error) => {
                agent.finished = true;
                self.fail(id, error, window, cx);
                return false;
            }
        }
        // An edit may have queued up behind one the user just settled.
        self.show_next_edit(id, window, cx);
        true
    }

    /// Put the next queued edit on screen, unless one is already there.
    fn show_next_edit(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent) = self.agent_run(id) else {
            return;
        };
        if agent.pending.is_some() {
            return;
        }
        let Some(edit) = agent.queue.pop_front() else {
            return;
        };
        if let Err(error) = self.show_edit(&edit, window, cx) {
            // Tell the agent, so it can try again.
            self.answer_edit(&edit.id, false, Some(&error), cx);
            self.show_next_edit(id, window, cx);
        }
    }

    fn show_edit(
        &mut self,
        edit: &EditRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let created = !edit.path.exists();
        let disk = std::fs::read_to_string(&edit.path).unwrap_or_default();
        let new = jig_ai::agent::apply_diff(&disk, &edit.diff).map_err(|error| {
            format!("Jig couldn't apply that edit: {error:#}. Read the file again.")
        })?;
        if created {
            std::fs::write(&edit.path, "").map_err(|error| error.to_string())?;
            self.refresh_tree(cx);
        }

        // Show the file, keeping the run alive across the tab switch.
        let on_screen = self
            .document()
            .path
            .as_deref()
            .is_some_and(|open| canonical(open) == canonical(&edit.path));
        if !on_screen {
            let run = self.run.take();
            self.open_file(&edit.path, window, cx);
            self.run = run;
        }
        if self.tab().dirty {
            return Err("The user has unsaved changes in this file; leave it alone.".into());
        }
        self.catch_up_with_disk(self.active, window, cx);

        let old = self.editor().text(cx);
        let preview = crate::diff::preview_edit(&old, &new, 0);
        let relative = self.relative(&edit.path);
        let Some(run) = self.run.as_mut() else {
            return Ok(());
        };
        run.preview = Some(Preview {
            range: preview.range.clone(),
        });
        run.bubble = Bubble::Preview {
            message: relative,
            removed: preview.removed,
            agent: true,
        };
        if let Some(agent) = run.agent.as_mut() {
            agent.pending = Some(PendingEdit {
                id: edit.id.clone(),
                path: edit.path.clone(),
                created,
            });
        }
        self.editor().clear_highlights(cx);
        let range = self
            .editor()
            .apply_edit(preview.range, preview.replacement, window, cx);
        self.editor().set_readonly(true, cx);
        let color = super::commands::added_color(cx);
        self.editor().highlight(
            preview
                .highlights
                .into_iter()
                .map(|range| (range, color))
                .collect(),
            cx,
        );
        self.editor().select(range.start..range.start, cx);
        if let Some(preview) = self.run.as_mut().and_then(|run| run.preview.as_mut()) {
            preview.range = range;
        }
        // The bubble follows the change once it's scrolled into view.
        cx.on_next_frame(window, |this, _, cx| {
            let anchor = this.floating_anchor(cx);
            if let Some(run) = this.run.as_mut() {
                run.anchor = anchor;
                cx.notify();
            }
        });
        cx.notify();
        Ok(())
    }

    /// Accept or reject the agent's edit on screen. Returns false when the
    /// preview isn't an agent edit, for the caller to handle.
    pub(super) fn settle_agent_edit(
        &mut self,
        accept: bool,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(run) = self.run.as_mut() else {
            return false;
        };
        let Some(pending) = run.agent.as_mut().and_then(|agent| agent.pending.take()) else {
            return false;
        };
        run.preview = None;
        if let Some(agent) = run.agent.as_mut() {
            run.bubble = Bubble::Running {
                label: agent.label.clone(),
                agent: true,
                started: agent.started,
                context: agent.context.clone(),
                step: Some(agent.step.clone()),
            };
            agent.step.set("Working".into());
            if accept {
                agent.touched.push(pending.path.clone());
            }
        }
        self.editor().clear_highlights(cx);
        self.editor().set_readonly(false, cx);
        if accept {
            // OpenCode writes the same text to disk.
            let text = self.editor().text(cx);
            let tab = self.tab_mut();
            tab.document.saved_text = text;
            tab.dirty = false;
        } else if let Some(window) = window {
            self.editor().undo(window, cx);
            if pending.created {
                let _ = std::fs::remove_file(&pending.path);
                let run = self.run.take();
                let tab = self.tab().id();
                self.remove_tab(tab, window, cx);
                self.run = run;
                self.refresh_tree(cx);
            }
        }
        self.answer_edit(&pending.id, accept, None, cx);
        cx.notify();
        true
    }

    fn answer_edit(&self, edit_id: &str, accept: bool, note: Option<&str>, cx: &mut Context<Self>) {
        let session = self
            .run
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .and_then(|agent| agent.session.clone());
        let (edit_id, note) = (edit_id.to_string(), note.map(str::to_string));
        if let Some(session) = session {
            cx.background_executor()
                .spawn(async move {
                    let _ = session.reply(&edit_id, accept, note.as_deref());
                })
                .detach();
        }
    }

    fn finish_agent(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let touched = self
            .run
            .as_mut()
            .and_then(|run| run.agent.take())
            .map(|mut agent| std::mem::take(&mut agent.touched))
            .unwrap_or_default();
        // Give OpenCode a moment to finish writing, then sync open tabs.
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(300))
                .await;
            this.update_in(cx, |this, window, cx| {
                for path in &touched {
                    if let Some(ix) = this.tab_for(path) {
                        this.catch_up_with_disk(ix, window, cx);
                    }
                }
                this.refresh_tree(cx);
            })
            .ok();
        })
        .detach();

        self.editor().clear_highlights(cx);
        if let Some(run) = self.run.as_mut() {
            run.bubble = Bubble::AgentDone(if text.is_empty() {
                "Done.".into()
            } else {
                text
            });
        }
        cx.notify();
    }

    /// If the tab at `ix` has no unsaved changes but its file changed on
    /// disk, show the file's new text.
    fn catch_up_with_disk(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tab = &mut self.tabs[ix];
        let Some(path) = tab.document.path.clone() else {
            return;
        };
        let Ok(disk) = std::fs::read_to_string(&path) else {
            return;
        };
        let text = tab.editor.text(cx);
        if tab.dirty || disk == text {
            return;
        }
        let (range, replacement) = crate::diff::changed_range(&text, &disk);
        tab.editor.apply_edit(range, replacement, window, cx);
        tab.document.saved_text = disk;
        tab.dirty = false;
    }

    fn relative(&self, path: &Path) -> String {
        let root = crate::project::root_for(path);
        path.strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }
}
