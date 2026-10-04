//! Agent commands: the command goes to a coding agent (OpenCode), which may
//! read and edit the whole project. Every edit it wants to make opens in the
//! editor as a normal preview, in its file's tab, and only goes through when
//! the user accepts it. The run is a conversation: once the agent answers,
//! the user can reply, and the agent carries on in the same session.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;
use jig_ai::PromptRequest;
use jig_ai::agent::{AgentEvent, AgentRequest, AgentSession, EditRequest};
use jig_commands::{Bubble, ChatBounds, ChatDrag, ChatEntry, ChatStatus, Conversation, LiveStep};
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
    /// The current turn is over (or stopped), so dropping the run leaves
    /// the agent alone.
    finished: bool,
    /// The project folder the agent works in.
    directory: PathBuf,
    /// Where the conversation started, for its header.
    file: String,
    entries: Vec<ChatEntry>,
    /// The reply box, shown between turns.
    pub(super) input: Entity<InputState>,
    scroll: ScrollHandle,
    /// Where the conversation is on screen, and while it's dragged, where
    /// the mouse holds it from its corner.
    bounds: ChatBounds,
    grab: Option<Point<Pixels>>,
    /// Dragged somewhere by the user: it stays there rather than following
    /// the agent's edits.
    moved: bool,
    _input_events: Subscription,
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
        name: Option<String>,
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
        // What the user asked, as the conversation's first line.
        let asked = match (&name, &request.comment) {
            (Some(name), Some(note)) => format!("{name}: {note}"),
            (Some(name), None) => name.clone(),
            (None, Some(note)) => format!("{}: {note}", request.instruction.trim()),
            (None, None) => request.instruction.trim().to_string(),
        };
        let agent = self.new_agent_run(name, asked, &path, window, cx);
        let run = self.run.as_mut().expect("run_command set up the run");
        run.agent = Some(agent);
        self.start_turn(id, jig_ai::agent::prompt(&request), window, cx);
    }

    /// A conversation that starts with the user asking `asked`, in the
    /// project around `path`. The agent isn't contacted yet.
    fn new_agent_run(
        &mut self,
        name: Option<String>,
        asked: String,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AgentRun {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Reply to the agent…"));
        let input_events =
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.reply_to_agent(window, cx);
                }
            });
        let context = self.context_names();
        let file = self.relative(path);
        AgentRun {
            session: None,
            pending: None,
            queue: VecDeque::new(),
            touched: Vec::new(),
            step: LiveStep::default(),
            label: name.unwrap_or_else(|| "Agent".into()),
            started: std::time::Instant::now(),
            context,
            finished: true,
            directory: crate::project::root_for(path),
            file,
            entries: vec![ChatEntry::User(asked)],
            input,
            scroll: ScrollHandle::new(),
            bounds: Default::default(),
            grab: None,
            moved: false,
            _input_events: input_events,
        }
    }

    /// An agent conversation between turns, without OpenCode behind it.
    #[cfg(test)]
    pub(super) fn open_test_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.document().path.clone().expect("a saved file");
        self.show_note(String::new(), window, cx);
        let agent = self.new_agent_run(None, "Hello".into(), &path, window, cx);
        if let Some(run) = self.run.as_mut() {
            run._task = None;
            run.agent = Some(agent);
        }
        cx.notify();
    }

    /// Send `prompt` to the agent, in the run's session once there is one,
    /// and follow what it does until it's done.
    fn start_turn(&mut self, id: u64, prompt: String, window: &mut Window, cx: &mut Context<Self>) {
        // The agent reads the disk, so it must see what's on screen.
        if self.tab().dirty
            && let Some(path) = self.document().path.clone()
        {
            self.save_to(&path, window, cx);
        }
        let Some(agent) = self.agent_run(id) else {
            return;
        };
        let session = agent.session.clone();
        let directory = agent.directory.clone();
        agent.step = LiveStep::default();
        agent.step.set(if session.is_some() {
            "Thinking".into()
        } else {
            "Starting the agent".into()
        });
        agent.started = std::time::Instant::now();
        agent.finished = false;
        let bubble = Bubble::Running {
            label: agent.label.clone(),
            agent: true,
            started: agent.started,
            context: agent.context.clone(),
            step: Some(agent.step.clone()),
        };

        let (sender, mut messages) = futures::channel::mpsc::unbounded();
        std::thread::spawn(move || {
            let send = |message| {
                let _ = sender.unbounded_send(message);
            };
            let result = (|| {
                let session = match session {
                    Some(session) => session,
                    None => {
                        let server = crate::agent::server()?;
                        let session = AgentSession::create(server, &directory)?;
                        send(Message::Started(session.clone()));
                        session
                    }
                };
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
        if let Some(run) = self.run.as_mut() {
            run.bubble = bubble;
            run._task = Some(task);
        }
        cx.notify();
    }

    /// Enter in the reply box: the agent takes the reply as its next turn.
    fn reply_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut().filter(|agent| agent.finished) else {
            return;
        };
        let text = agent.input.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        agent
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        agent.entries.push(ChatEntry::User(text.clone()));
        agent.scroll.scroll_to_bottom();
        // Back to the code, where the agent's edits are reviewed.
        self.editor().focus(window, cx);
        self.start_turn(id, text, window, cx);
    }

    /// Esc while the agent works: stop the turn but keep the conversation,
    /// so the user can say what to do instead. Returns false when there is
    /// no turn to stop.
    pub(super) fn stop_agent_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(run) = self.run.as_mut() else {
            return false;
        };
        let Some(agent) = run.agent.as_mut().filter(|agent| !agent.finished) else {
            return false;
        };
        agent.finished = true;
        agent.queue.clear();
        if let Some(session) = agent.session.clone() {
            std::thread::spawn(move || session.abort());
        }
        // Whatever the agent still sends is for a turn that's over.
        run._task = None;
        self.end_turn(ChatEntry::Note("Stopped.".into()), window, cx);
        true
    }

    #[cfg(test)]
    pub(super) fn agent_entries(&self) -> Vec<ChatEntry> {
        self.run
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map(|agent| agent.entries.clone())
            .unwrap_or_default()
    }

    /// Pressed on the conversation's header: a drag may follow.
    fn grab_agent_chat(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let Some(agent) = run.agent.as_mut() else {
            return;
        };
        // From where it's drawn, which may differ from the anchor when it
        // was pushed in to fit the window.
        let origin = agent.bounds.get().origin;
        agent.grab = Some(event.position - origin);
        run.anchor = origin;
        cx.stop_propagation();
    }

    /// The conversation follows the mouse while its header is dragged.
    pub(super) fn drag_agent_chat(
        &mut self,
        event: &DragMoveEvent<ChatDrag>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = self.run.as_mut() else {
            return;
        };
        let Some(agent) = run.agent.as_mut() else {
            return;
        };
        let Some(grab) = agent.grab else {
            return;
        };
        run.anchor = event.event.position - grab;
        agent.moved = true;
        cx.notify();
    }

    /// The conversation, while the run is an agent's.
    pub(super) fn render_agent_chat(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let run = self.run.as_ref()?;
        let agent = run.agent.as_ref()?;
        let status = match &run.bubble {
            Bubble::Running {
                started,
                step: Some(step),
                ..
            } => ChatStatus::Working {
                started: *started,
                step: step.clone(),
            },
            Bubble::Preview {
                message, removed, ..
            } => ChatStatus::Reviewing {
                path: message.clone(),
                removed: removed.clone(),
            },
            _ => ChatStatus::Waiting,
        };
        Some(
            Conversation::new(
                format!("{} · in {}", agent.label, agent.file),
                agent.entries.clone(),
                status,
                agent.input.clone(),
                agent.scroll.clone(),
                agent.bounds.clone(),
            )
            .on_grab(cx.listener(|this, event, _, cx| this.grab_agent_chat(event, cx)))
            .into_any_element(),
        )
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
                let error = super::commands::shorten(&error);
                self.end_turn(ChatEntry::Error(error), window, cx);
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
            if let Some(run) = this
                .run
                .as_mut()
                .filter(|run| !run.agent.as_ref().is_some_and(|agent| agent.moved))
            {
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
            let path = match &run.bubble {
                Bubble::Preview { message, .. } => message.clone(),
                _ => pending.path.to_string_lossy().into_owned(),
            };
            agent.entries.push(ChatEntry::Edit {
                path,
                accepted: accept,
            });
            agent.scroll.scroll_to_bottom();
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
        let text = if text.is_empty() {
            "Done.".to_string()
        } else {
            text
        };
        if let Some(run) = self.run.as_mut() {
            run.bubble = Bubble::AgentDone(text.clone());
        }
        self.end_turn(ChatEntry::Agent(text), window, cx);
    }

    /// Close the agent's turn with `entry` and open the reply box.
    fn end_turn(&mut self, entry: ChatEntry, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent) = self.run.as_mut().and_then(|run| run.agent.as_mut()) else {
            return;
        };
        let touched = std::mem::take(&mut agent.touched);
        agent.entries.push(entry);
        agent.scroll.scroll_to_bottom();
        let input = agent.input.clone();
        if let Some(run) = self.run.as_mut()
            && run.bubble.is_running()
        {
            run.bubble = Bubble::Message(String::new());
        }
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
        input.update(cx, |input, cx| input.focus(window, cx));
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
