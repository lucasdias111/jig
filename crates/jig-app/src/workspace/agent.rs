//! The agent: the request goes to a coding agent (OpenCode by default),
//! which may read and edit the whole project. Every edit it wants to make opens in the
//! editor as a normal preview, in its file's tab, and only goes through when
//! the user accepts it; shell commands, the web and subagents wait for a
//! go-ahead too. The run is a conversation: once the agent answers, the
//! user can reply, and the agent carries on in the same session. Earlier
//! conversations are listed in the ⌘K input and can be picked up again.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use futures::StreamExt as _;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{InlineToken, InputContent, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_ai::agent::acp::SignInNeeded;
use jig_ai::agent::harness::Session;
use jig_ai::agent::read_only::command_only_reads;
use jig_ai::agent::{
    AgentEvent, AgentRequest, Asking, CodeReference, EditRequest, PermissionRequest,
    QuestionRequest, Turn,
};
use jig_commands::{
    AgentCommandInfo, Bubble, ChatDrag, ChatEntry, ChatFrame, ChatModels, ChatResize, ChatStatus,
    CommandPalette, Conversation, DrawingAction, LiveStep, ModelChoice, ModelMenu,
    PastConversation,
};
use jig_editor::EditorHandle;
use serde_json::Value;

mod agent_mode;
mod conversations;
mod edits;
mod events;
mod frame;
mod map_actions;
mod models;
mod requests;
#[cfg(test)]
mod testing;
mod turns;
use agent_mode::*;

pub(super) mod catalog;

use super::commands::{CommandRun, Preview};
use super::tabs::canonical;
use super::{EditAgents, OpenAgentConversations, OpenCommand, SendSelectionToAgent, Workspace};
use catalog::{AgentCatalog, CatalogLoaded, ConversationCatalog};
use jig_map::{Action, only_reads};
use jig_planner::link_files;

/// Why a conversation can't start.
const NO_PROJECT: &str =
    "Open the project's folder, or save the file, so the agent knows which project it's in.";

/// What the agent's thread reports to the window.
enum Message {
    Started(Session),
    /// An earlier conversation, picked up again.
    Resumed(Session, Vec<Turn>),
    /// The models an agent (by key) says it offers.
    Models(String, jig_ai::agent::acp::AgentModels),
    Event(AgentEvent),
    /// The agent wants the user to sign in first.
    SignIn(SignInNeeded),
    Failed(String),
}

/// A sign-in the conversation offers, and whether its terminal was opened.
struct PendingSignIn {
    needed: SignInNeeded,
    opened: bool,
}

/// What went wrong on the agent's thread, as a message for the window.
fn failure(error: anyhow::Error) -> Message {
    match error.downcast_ref::<SignInNeeded>() {
        Some(needed) => Message::SignIn(needed.clone()),
        None => Message::Failed(format!("{error:#}")),
    }
}

/// What a turn sends the agent.
#[derive(Clone)]
pub(super) enum TurnInput {
    Prompt(String),
    /// An OpenCode command or skill, with its arguments.
    Command {
        name: String,
        arguments: String,
    },
}

/// Something the agent waits on the user for.
enum Request {
    Edit(EditRequest),
    Permission(PermissionRequest),
    Question(QuestionRequest),
}

/// What the window keeps for the agent from one conversation to the next.
#[derive(Default)]
pub(super) struct AgentState {
    /// The agent and model picked in ⌘K for new conversations; `None`
    /// follows the settings.
    pub(super) pick: Option<jig_agent::agent::Pick>,
    recent_scroll: ScrollHandle,
    /// An unsent project message; opening Agent does not create a conversation.
    home_draft: Option<(PathBuf, Entity<TextareaState>)>,
    /// The docked conversation's width, when the last one was left docked:
    /// the next opens the same way.
    pub(super) dock: Option<Pixels>,
    /// The docked conversation's edge is being dragged.
    resizing_dock: bool,
}

/// The agent side of a [`CommandRun`].
pub(super) struct AgentRun {
    /// `None` until the session is set up.
    session: Option<Session>,
    /// What's on screen, awaiting the user.
    pending: Option<Pending>,
    /// Requests that came in while another was on screen.
    queue: VecDeque<Request>,
    /// An earlier conversation is being fetched.
    loading: bool,
    /// The last turn asked, to try again after signing in.
    last_input: Option<TurnInput>,
    /// The agent and model it runs on: ⌘K's pick when it started, else the
    /// settings'. The conversation keeps its agent; its header changes the
    /// model.
    pub(super) pick: Option<jig_agent::agent::Pick>,
    /// The header's list of models is open.
    models_open: bool,
    /// The agent wants the user to sign in first.
    sign_in: Option<PendingSignIn>,
    /// Kinds of request (their titles, e.g. "Run a command") allowed for
    /// the rest of the conversation, for agents that can't remember that
    /// themselves.
    always_allowed: Vec<String>,
    /// What the user answered a question with that the agent can't hear
    /// until its turn is over: sent as the next message then.
    follow_up: Option<String>,
    /// ⌘↩ on an edit: the agent's edits are written without review for the
    /// rest of the conversation, each still listed in it.
    accept_edits: bool,
    /// What the conversation is called in OpenCode: what was first asked.
    title: String,
    /// Files the agent changed, to catch up with the disk when it's done
    /// (OpenCode may format a file after writing it).
    touched: Vec<PathBuf>,
    step: LiveStep,
    pub(super) label: String,
    started: std::time::Instant,
    context: Vec<String>,
    /// The current turn is over (or stopped), so dropping the run leaves
    /// the agent alone.
    pub(super) finished: bool,
    /// The project folder the agent works in.
    directory: PathBuf,
    /// The project's folder name, for its header.
    project: String,
    pub(super) entries: Vec<ChatEntry>,
    /// The reply box, shown between turns.
    pub(super) input: Entity<TextareaState>,
    page_focus: FocusHandle,
    /// The code sent over from the editor, by the id of its token in the
    /// reply box, sent with the reply that still names it.
    references: Vec<(SharedString, CodeReference)>,
    pub(super) scroll: ScrollHandle,
    /// The conversation's size, and where it is on screen.
    frame: ChatFrame,
    /// While it's dragged, where the mouse holds it from its corner.
    grab: Option<Point<Pixels>>,
    /// While it's resized: where the mouse started, and the width and
    /// transcript height then.
    resize: Option<(Point<Pixels>, Size<Pixels>)>,
    /// Dragged somewhere by the user: it stays there rather than following
    /// the agent's edits.
    moved: bool,
    /// The Map action this conversation is, whose answers the Map reads
    /// back as proposals; it may run commands that only read.
    pub(super) map_action: Option<Action>,
    /// This turn's answer to its Map action, as the agent gave it to
    /// `map_propose_domains`: read in place of its words when it's done.
    pub(super) proposed: Option<Value>,
    /// What it drew on the Map, by their place here.
    pub(super) drawings: Vec<super::map_agent::Drawn>,
}

enum Pending {
    Edit(PendingEdit),
    Permission(PermissionRequest),
    Question {
        request: QuestionRequest,
        /// Answers to the questions before the one shown.
        answers: Vec<String>,
    },
}

struct PendingEdit {
    id: String,
    path: PathBuf,
    /// The file didn't exist; it is removed again if the edit is rejected.
    created: bool,
    /// The agent left the writing to Jig.
    jig_writes: bool,
    /// The buffer's text before the edit went in, to put back if it's
    /// rejected.
    before: String,
}

impl AgentRun {
    /// Where the conversation was last drawn, in the window.
    pub(super) fn frame_bounds(&self) -> jig_commands::chat::ChatBounds {
        self.frame.bounds.clone()
    }

    /// The project folder it works in.
    pub(super) fn directory(&self) -> &Path {
        &self.directory
    }

    /// The transcript's row for subagent `id`: its description, what it
    /// did and its report, added when it's first heard of.
    fn subagent(&mut self, id: &str) -> (&mut String, &mut Vec<String>, &mut Option<String>) {
        let found = self.entries.iter().position(
            |entry| matches!(entry, ChatEntry::Subagent { id: known, .. } if known == id),
        );
        let at = found.unwrap_or_else(|| {
            self.entries.push(ChatEntry::Subagent {
                id: id.to_string(),
                description: "Subagent".into(),
                calls: Vec::new(),
                report: None,
            });
            self.scroll.scroll_to_bottom();
            self.entries.len() - 1
        });
        match &mut self.entries[at] {
            ChatEntry::Subagent {
                description,
                calls,
                report,
                ..
            } => (description, calls, report),
            _ => unreachable!("found as a subagent"),
        }
    }
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
    /// The file of the agent's edit under review, when it isn't the tab on
    /// screen.
    pub(super) fn agent_edit_elsewhere(&self) -> Option<PathBuf> {
        let agent = self.chat.as_ref()?.agent.as_ref()?;
        let Some(Pending::Edit(edit)) = agent.pending.as_ref() else {
            return None;
        };
        let here = self.document().path.as_deref().map(canonical);
        (here.as_deref() != Some(canonical(&edit.path).as_path())).then(|| edit.path.clone())
    }

    /// Back to the tab where the agent's edit waits for review. False when
    /// it isn't waiting in another tab.
    pub(super) fn show_agent_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.mode == super::mode::Mode::Agent && self.agent_edit_here() {
            self.focus_editor(window, cx);
            return true;
        }
        let Some(ix) = self
            .agent_edit_elsewhere()
            .and_then(|path| self.tab_for(&path))
        else {
            return false;
        };
        self.activate(ix, window, cx);
        true
    }

    /// Esc for the agent conversation: back to its edit waiting in another
    /// tab, else reject the edit here, else stop its turn, else close it.
    /// False with nothing to do.
    pub(super) fn escape_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.show_agent_edit(window, cx)
            || (self.agent_edit_here() && self.settle_agent_edit(false, Some(window), cx))
            || self.stop_agent_turn(window, cx)
        {
            return true;
        }
        if self.chat.is_none() {
            return false;
        }
        if self.mode == super::mode::Mode::Agent {
            self.focus_agent_mode(window, cx);
            return true;
        }
        self.close_chat(window, cx);
        // The agent's reply box may have had the keyboard.
        self.focus_main(window, cx);
        true
    }

    /// Esc in a sidebar view with nothing of its own left to close: back to
    /// the code, and the agent conversation, often docked beside the
    /// sidebar, closes once it's idle. A working agent, or one waiting on
    /// the user, is left alone: stopping it is for Esc in the code.
    pub(super) fn leave_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_main(window, cx);
        let idle = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| {
                agent.finished && agent.pending.is_none() && agent.queue.is_empty()
            });
        if idle {
            self.close_chat(window, cx);
        }
    }

    /// The conversation offers ways to sign in to the agent.
    fn signing_in(&self) -> bool {
        self.chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| agent.sign_in.is_some())
    }

    /// Esc while the agent works: stop the turn but keep the conversation,
    /// so the user can say what to do instead. Returns false when there is
    /// no turn to stop.
    pub(super) fn stop_agent_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // An edit still in the buffer goes, as when rejected, rather than
        // staying there read-only.
        if self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| !agent.finished && matches!(agent.pending, Some(Pending::Edit(_))))
        {
            self.settle_agent_edit(false, Some(window), cx);
        }
        let Some(run) = self.chat.as_mut() else {
            return false;
        };
        let Some(agent) = run.agent.as_mut().filter(|agent| !agent.finished) else {
            return false;
        };
        agent.finished = true;
        agent.queue.clear();
        agent.pending = None;
        agent.follow_up = None;
        if let Some(session) = agent.session.clone() {
            std::thread::spawn(move || session.abort());
        }
        // Whatever the agent still sends is for a turn that's over.
        run._task = None;
        self.end_turn(ChatEntry::Note("Stopped.".into()), window, cx);
        true
    }

    fn relative(&self, path: &Path) -> String {
        let root = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map(|agent| agent.directory.clone())
            .filter(|root| path.starts_with(root))
            .unwrap_or_else(|| jig_kit::project::root_for(path));
        path.strip_prefix(&root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned()
    }

    /// The project an agent conversation works in: the folder open in the
    /// sidebar when the file is in it (or there's no saved file), else the
    /// file's repository. The same from every file, so a conversation and
    /// its history belong to the project, not the file it started from.
    pub(super) fn agent_project(&self, cx: &App) -> Option<PathBuf> {
        let open = self.project_root(cx);
        match self.document().path.as_deref() {
            Some(path) => Some(
                open.filter(|root| path.starts_with(root))
                    .unwrap_or_else(|| jig_kit::project::root_for(path)),
            ),
            None => open,
        }
    }
}

/// A line of an earlier conversation, as the transcript shows it.
pub(super) fn chat_entry(turn: Turn) -> ChatEntry {
    match turn {
        Turn::User(text) => ChatEntry::User(text),
        Turn::Agent(text) => ChatEntry::Agent(text),
        Turn::Edit { path, accepted } => ChatEntry::Edit {
            path,
            accepted,
            diff: Vec::new(),
        },
        Turn::Did(text) => ChatEntry::Did(text),
        // Drawings are read back with what they drew (`map_agent`).
        Turn::Drew { tool, .. } => ChatEntry::Did(format!("Drew with {tool}")),
    }
}

#[cfg(test)]
mod tests;
