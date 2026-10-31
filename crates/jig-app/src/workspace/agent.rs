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
    pub(super) fn run_agent(
        &mut self,
        id: u64,
        name: Option<String>,
        request: jig_ai::agent::Prompt,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(directory) = self.agent_project(cx) else {
            self.chat = None;
            return self.show_note(NO_PROJECT.into(), window, cx);
        };
        // What the user asked, as the conversation's first line.
        let asked = name
            .clone()
            .unwrap_or_else(|| request.instruction.trim().to_string());
        let agent = self.new_agent_run(name, asked, directory, window, cx);
        let run = self
            .chat
            .as_mut()
            .expect("start_conversation set up the conversation");
        run.agent = Some(agent);
        let anchor = self.agent_anchor(window);
        if let Some(run) = self.chat.as_mut() {
            run.anchor = anchor;
        }
        let prompt = jig_ai::agent::prompt(&request);
        self.start_turn(id, TurnInput::Prompt(prompt), window, cx);
    }

    /// Start a conversation with an OpenCode command or skill, e.g.
    /// "/review main".
    pub(super) fn run_agent_command(
        &mut self,
        name: String,
        arguments: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.start_agent_run(window, cx) else {
            return;
        };
        let asked = format!("/{name} {arguments}").trim().to_string();
        let directory = self.agent_project(cx).expect("start_agent_run checked");
        let agent = self.new_agent_run(Some(format!("/{name}")), asked, directory, window, cx);
        if let Some(run) = self.chat.as_mut() {
            run.agent = Some(agent);
        }
        self.start_turn(id, TurnInput::Command { name, arguments }, window, cx);
    }

    /// Start the conversation for a Map action in project `directory`:
    /// named after the action, the agent asked `instruction`, beside the
    /// Map rather than at the code.
    pub(super) fn start_map_action(
        &mut self,
        action: Action,
        instruction: String,
        directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agent_edit_here() {
            self.end_map_action(cx);
            return self.show_note(
                "Accept or reject the agent's edit first.".into(),
                window,
                cx,
            );
        }
        self.close_chat(window, cx);
        self.next_run_id += 1;
        let id = self.next_run_id;
        let title = action.title();
        let width = px(jig_commands::chat::WIDTH);
        let anchor = point(
            (window.viewport_size().width - width - px(24.)).max(px(8.)),
            px(super::TITLE_BAR_HEIGHT + 48.),
        );
        let mut agent = self.new_agent_run(Some(title.clone()), title, directory, window, cx);
        agent.map_action = Some(action);
        self.chat = Some(CommandRun {
            id,
            bubble: Bubble::Message(String::new()),
            anchor,
            preview: None,
            agent: Some(agent),
            _task: None,
        });
        self.start_turn(id, TurnInput::Prompt(instruction), window, cx);
    }

    /// The Map's action is over without an answer to read.
    fn end_map_action(&self, cx: &mut App) {
        if let Some(pane) = self.map.as_ref() {
            pane.view.update(cx, |view, cx| view.action_ended(cx));
        }
    }

    /// The agent answered a Map action, with `map_propose_domains` (its
    /// `call`) or in its words `text`: the Map reads the answer as a
    /// proposal and shows it at Domain, or the conversation says why it
    /// couldn't.
    fn answer_map_action(
        &mut self,
        action: Action,
        text: &str,
        call: Option<&Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.map.as_ref().map(|pane| pane.view.clone()) else {
            return;
        };
        let read = view.update(cx, |view, cx| view.answer(action, text, call, cx));
        let proposed = read.is_ok();
        let entry = match read {
            Ok(()) => ChatEntry::Note(
                "It's on the Map as a proposal: keep it there (⌘↩), edit it, or reply here to redo parts."
                    .into(),
            ),
            Err(error) => ChatEntry::Error(format!(
                "The Map couldn't read domains from that: {error} Reply to ask again."
            )),
        };
        if let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) {
            agent.entries.push(entry);
            agent.scroll.scroll_to_bottom();
        }
        if proposed {
            self.map_at(jig_map::Level::Domain, window, cx);
        }
        cx.notify();
    }

    /// Pick up the earlier conversation `session_id` where it left off.
    pub(super) fn resume_agent(
        &mut self,
        session_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.start_agent_run(window, cx) else {
            return;
        };
        let title = self
            .catalog
            .read(cx)
            .catalog()
            .and_then(|catalog| {
                catalog
                    .conversations
                    .iter()
                    .find(|conversation| conversation.id == session_id)
            })
            .map(conversation_title)
            .unwrap_or_else(|| "Earlier conversation".into());
        let directory = self.agent_project(cx).expect("start_agent_run checked");
        let mut agent = self.new_agent_run(Some(title.clone()), title, directory, window, cx);
        agent.entries.clear();
        agent.loading = true;
        let directory = agent.directory.clone();
        let choice = jig_agent::agent::choice_for(agent.pick.as_ref(), cx);
        if let Some(run) = self.chat.as_mut() {
            run.agent = Some(agent);
        }

        let (sender, mut messages) = futures::channel::mpsc::unbounded();
        let resume = move || {
            let send = |message| {
                let _ = sender.unbounded_send(message);
            };
            let result = (|| {
                let choice = choice.map_err(anyhow::Error::msg)?;
                let backend = jig_agent::agent::connect(&choice)?;
                let session = backend.open(&directory, &session_id)?;
                let turns = session.history()?;
                if let Some(model) = choice.acp_model() {
                    session.use_model(model)?;
                }
                if let Some(models) = backend.models() {
                    send(Message::Models(choice.key().to_string(), models));
                }
                anyhow::Ok(Message::Resumed(session, turns))
            })();
            send(result.unwrap_or_else(failure));
        };
        // On a thread of its own, as a turn is (see `start_turn`).
        #[cfg(not(test))]
        std::thread::spawn(resume);
        #[cfg(test)]
        cx.background_executor()
            .spawn(async move { resume() })
            .detach();
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
        if let Some(run) = self.chat.as_mut() {
            run._task = Some(task);
        }
        cx.notify();
    }

    /// Set up an empty run for a conversation at the cursor. `None`, with
    /// the reason shown, when the file isn't saved anywhere yet.
    fn start_agent_run(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<u64> {
        if self.mode == super::mode::Mode::Agent && self.agent_busy() {
            self.show_note(
                "Finish or stop the current conversation first.".into(),
                window,
                cx,
            );
            return None;
        }
        self.close_chat(window, cx);
        self.next_run_id += 1;
        let id = self.next_run_id;
        let anchor = self.agent_anchor(window);
        self.chat = Some(CommandRun {
            id,
            bubble: Bubble::Message(String::new()),
            anchor,
            preview: None,
            agent: None,
            _task: None,
        });
        if self.agent_project(cx).is_none() {
            self.chat = None;
            self.show_note(NO_PROJECT.into(), window, cx);
            return None;
        }
        Some(id)
    }

    /// Close the conversation on screen. An edit of its still under review
    /// is undone first: nothing reaches the disk unreviewed. The
    /// conversation stays with the agent, in the history.
    pub(super) fn close_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settle_agent_edit(false, Some(window), cx);
        if self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| agent.map_action.is_some() && !agent.finished)
        {
            self.end_map_action(cx);
        }
        if self.chat.take().is_some() {
            self.editor().clear_highlights(cx);
            self.sync_map_drawings(cx);
            cx.notify();
        }
    }

    /// ⌥⌘K, or the conversation's history button: the ⌘K input, which
    /// lists the earlier conversations.
    pub(super) fn open_agent_conversations(
        &mut self,
        _: &OpenAgentConversations,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_none() {
            self.open_command(&OpenCommand, window, cx);
        }
    }

    /// What ⌘K's model chip offers: each installed agent with its models
    /// (OpenCode's from the providers, the others' as they last said).
    pub(super) fn model_menu(&self, cx: &App) -> ModelMenu {
        let offered = jig_agent::providers::offered_by_provider(cx);
        let picked = self.picked_agent(cx);
        let pick = self
            .agent
            .pick
            .clone()
            .unwrap_or_else(|| jig_agent::agent::Pick::from_settings(cx));
        let mut agent = ModelMenu::default();
        for listed in jig_agent::agents::registry().agents.iter() {
            // Agents not installed would only fail; the chosen one shows anyway.
            if listed.key != picked.key && !jig_agent::agents::status(listed).1 {
                continue;
            }
            let own: Vec<(String, String)> = match listed.kind {
                jig_agent::agents::Kind::OpenCode => offered
                    .iter()
                    .map(|(id, provider, model)| (id.clone(), format!("{provider} · {model}")))
                    .collect(),
                jig_agent::agents::Kind::Acp => jig_agent::agents::known_models(&listed.key)
                    .map(|models| models.options)
                    .unwrap_or_default(),
            };
            if own.is_empty() {
                agent.choices.push(ModelChoice {
                    agent: Some(listed.key.clone()),
                    group: listed.name.clone(),
                    name: "Default model".into(),
                    model: None,
                    label: listed.name.clone(),
                });
            }
            for (model, name) in own {
                agent.choices.push(ModelChoice {
                    agent: Some(listed.key.clone()),
                    group: listed.name.clone(),
                    label: format!("{} · {name}", listed.name),
                    name,
                    model: Some(model),
                });
            }
        }
        agent.current = Some(position_or_push(
            &mut agent.choices,
            Some(&pick.agent),
            pick.model.as_deref(),
            &picked.name,
            pick.model.as_deref().unwrap_or("Default model"),
        ));
        agent
    }

    /// The palette's lists again, as the providers' models arrive.
    pub(super) fn refresh_palette_models(&self, cx: &mut Context<Self>) {
        let Some(palette) = self.palette.as_ref().map(|palette| palette.view.clone()) else {
            return;
        };
        let menu = self.model_menu(cx);
        palette.update(cx, |palette, cx| palette.set_models(menu, cx));
    }

    /// ⌘K's chip picked `choice` for the window: the agent and model new
    /// conversations run.
    pub(super) fn pick_model(
        &mut self,
        choice: ModelChoice,
        palette: Entity<CommandPalette>,
        cx: &mut Context<Self>,
    ) {
        let Some(agent) = choice.agent else {
            return;
        };
        let pick = jig_agent::agent::Pick {
            agent,
            model: choice.model,
        };
        // The settings' own choice needs no pick.
        self.agent.pick = (pick != jig_agent::agent::Pick::from_settings(cx)).then_some(pick);
        if self.mode == super::mode::Mode::Agent {
            let pick = self
                .agent
                .pick
                .clone()
                .unwrap_or_else(|| jig_agent::agent::Pick::from_settings(cx));
            let label = self.picked_agent(cx).name.clone();
            if let Some(agent) = self
                .chat
                .as_mut()
                .and_then(|run| run.agent.as_mut())
                .filter(|agent| {
                    agent.entries.is_empty() && agent.session.is_none() && agent.finished
                })
            {
                agent.pick = Some(pick);
                agent.label = label;
            }
        }
        // Its conversations and commands, in place of the last agent's.
        self.load_agent_catalog(Some(palette), cx);
        cx.notify();
    }

    /// What the open conversation can run on: its agent's models, each as
    /// the model value to use, and as its header lists them.
    fn conversation_models(&self, cx: &App) -> (Vec<Option<String>>, ChatModels) {
        let Some(pick) = self
            .chat
            .as_ref()
            .and_then(|chat| chat.agent.as_ref())
            .and_then(|agent| agent.pick.clone())
        else {
            return (Vec::new(), ChatModels::default());
        };
        let open = self
            .chat
            .as_ref()
            .and_then(|chat| chat.agent.as_ref())
            .is_some_and(|agent| agent.models_open);
        let agent = jig_agent::agents::named(&pick.agent);
        let listed: Vec<(String, String)> = match agent.kind {
            jig_agent::agents::Kind::OpenCode => jig_agent::providers::offered(cx),
            jig_agent::agents::Kind::Acp => jig_agent::agents::known_models(&agent.key)
                .map(|models| models.options)
                .unwrap_or_default(),
        };
        let mut values: Vec<Option<String>> = listed
            .iter()
            .map(|(value, _)| Some(value.clone()))
            .collect();
        let mut labels: Vec<String> = listed.into_iter().map(|(_, label)| label).collect();
        // On its default, or on a model no longer listed: say so.
        let in_use = pick
            .model
            .clone()
            .or_else(|| jig_agent::agents::known_models(&agent.key).map(|models| models.current));
        let current = match values.iter().position(|value| *value == in_use) {
            Some(at) => at,
            None => {
                values.push(in_use.clone());
                labels.push(in_use.unwrap_or_else(|| "Default model".into()));
                labels.len() - 1
            }
        };
        (
            values,
            ChatModels {
                options: labels,
                current: Some(current),
                open,
            },
        )
    }

    fn toggle_conversation_models(&mut self, cx: &mut Context<Self>) {
        if let Some(agent) = self.chat.as_mut().and_then(|chat| chat.agent.as_mut()) {
            agent.models_open = !agent.models_open;
            cx.notify();
        }
    }

    /// The header's model picked: the conversation goes on with it. An ACP
    /// agent is switched now; OpenCode takes it with the next message.
    fn pick_conversation_model(&mut self, index: usize, cx: &mut Context<Self>) {
        let (values, models) = self.conversation_models(cx);
        let Some(model) = values.get(index).cloned() else {
            return;
        };
        let Some(agent) = self.chat.as_mut().and_then(|chat| chat.agent.as_mut()) else {
            return;
        };
        agent.models_open = false;
        let Some(pick) = agent.pick.as_mut() else {
            return;
        };
        if pick.model == model {
            return cx.notify();
        }
        pick.model = model.clone();
        let label = models.options.get(index).cloned().unwrap_or_default();
        agent
            .entries
            .push(ChatEntry::Note(format!("Now on {label}.")));
        agent.scroll.scroll_to_bottom();
        if let (Some(session), Some(model)) = (agent.session.clone(), model) {
            cx.background_executor()
                .spawn(async move {
                    let _ = session.use_model(&model);
                })
                .detach();
        }
        cx.notify();
    }

    /// The agent new conversations run: picked in ⌘K, else in Settings.
    pub(super) fn picked_agent(&self, cx: &App) -> std::sync::Arc<jig_agent::agents::Agent> {
        match &self.agent.pick {
            Some(pick) => jig_agent::agents::named(&pick.agent),
            None => jig_agent::agents::chosen(cx),
        }
    }

    /// Open `agents.toml`, starting it with a commented example.
    pub(super) fn edit_agents(
        &mut self,
        _: &EditAgents,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = jig_agent::agents::user_path() else {
            return;
        };
        if !path.exists() {
            let created = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, jig_agent::agents::USER_TEMPLATE));
            if let Err(error) = created {
                self.show_error(
                    &format!("Couldn't create {}: {error}", path.display()),
                    window,
                    cx,
                );
                return;
            }
        }
        self.open_file(&path, window, cx);
    }

    /// Fill the ⌘K input's lists: what's known at once, then what the agent
    /// says now. Without a palette, just learn the commands, so a
    /// reply can run one.
    pub(super) fn load_agent_catalog(
        &mut self,
        palette: Option<Entity<CommandPalette>>,
        cx: &mut Context<Self>,
    ) {
        let Some(directory) = self.agent_project(cx) else {
            return;
        };
        let agent = self.picked_agent(cx).key.clone();
        if let Some(catalog) = self
            .catalog
            .read(cx)
            .for_project(&directory, &agent)
            .cloned()
            && let Some(palette) = &palette
        {
            show_catalog(&catalog, palette, cx);
        }
        let choice = jig_agent::agent::choice_for(self.agent.pick.as_ref(), cx);
        self.catalog.update(cx, |catalog, cx| {
            catalog.load(directory, agent, choice, palette, cx)
        });
        cx.notify();
    }

    /// A load of the catalog finished: keep it, and fill the ⌘K input that
    /// asked, unless the project or agent has changed since.
    pub(super) fn on_catalog_loaded(
        &mut self,
        catalog: Entity<ConversationCatalog>,
        loaded: &CatalogLoaded,
        cx: &mut Context<Self>,
    ) {
        if self.agent_project(cx).as_ref() != Some(&loaded.directory)
            || self.picked_agent(cx).key != loaded.agent
        {
            cx.notify();
            return;
        }
        match &loaded.result {
            Err(error) => {
                catalog.update(cx, |catalog, cx| catalog.failed(error.clone(), cx));
            }
            Ok(found) => {
                if let Some(palette) = &loaded.palette {
                    show_catalog(found, palette, cx);
                }
                let found = found.clone();
                catalog.update(cx, |catalog, cx| catalog.keep(found, cx));
            }
        }
        cx.notify();
    }

    /// A conversation that starts with the user asking `asked`, in the
    /// project around `path`. The agent isn't contacted yet.
    pub(super) fn new_agent_run(
        &mut self,
        name: Option<String>,
        asked: String,
        directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AgentRun {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Reply to the agent…")
                .auto_grow(1, 8)
                .soft_wrap(true)
        });
        let context = self.context_names();
        let project = directory
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| directory.display().to_string());
        AgentRun {
            session: None,
            pending: None,
            queue: VecDeque::new(),
            loading: false,
            last_input: None,
            pick: Some(
                self.agent
                    .pick
                    .clone()
                    .unwrap_or_else(|| jig_agent::agent::Pick::from_settings(cx)),
            ),
            models_open: false,
            sign_in: None,
            always_allowed: Vec::new(),
            follow_up: None,
            accept_edits: false,
            title: asked.clone(),
            touched: Vec::new(),
            step: LiveStep::default(),
            label: name.unwrap_or_else(|| self.picked_agent(cx).name.clone()),
            started: std::time::Instant::now(),
            context,
            finished: true,
            directory,
            project,
            entries: vec![ChatEntry::User(asked)],
            input,
            page_focus: cx.focus_handle(),
            references: Vec::new(),
            scroll: ScrollHandle::new(),
            frame: match self.agent.dock {
                Some(width) => ChatFrame {
                    width,
                    pinned: true,
                    ..ChatFrame::default()
                },
                None => ChatFrame::default(),
            },
            grab: None,
            resize: None,
            moved: false,
            map_action: None,
            proposed: None,
            drawings: Vec::new(),
        }
    }

    /// An agent conversation between turns, without OpenCode behind it.
    #[cfg(test)]
    pub(super) fn open_test_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.start_agent_run(window, cx).expect("a project");
        let directory = self.agent_project(cx).expect("a project");
        let agent = self.new_agent_run(None, "Hello".into(), directory, window, cx);
        if let Some(run) = self.chat.as_mut().filter(|run| run.id == id) {
            run.agent = Some(agent);
        }
        let anchor = self.agent_anchor(window);
        if let Some(run) = self.chat.as_mut() {
            run.anchor = anchor;
        }
        cx.notify();
    }

    /// Hand the open test conversation a request, as if the agent had sent
    /// it mid-turn.
    #[cfg(test)]
    pub(super) fn test_agent_request(
        &mut self,
        event: AgentEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.chat.as_ref().map_or(0, |run| run.id);
        if let Some(agent) = self.agent_run(id) {
            agent.finished = false;
        }
        self.on_agent_message(id, Message::Event(event), window, cx);
    }

    /// As if the agent had said the user must sign in first.
    #[cfg(test)]
    pub(super) fn test_sign_in(
        &mut self,
        needed: SignInNeeded,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = self.chat.as_ref().map_or(0, |run| run.id);
        self.on_agent_message(id, Message::SignIn(needed), window, cx);
    }

    /// How far the transcript is scrolled down, and how far it could be.
    #[cfg(test)]
    pub(super) fn agent_scroll(&self) -> (Pixels, Pixels) {
        let agent = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .unwrap();
        (-agent.scroll.offset().y, agent.scroll.max_offset().y)
    }

    /// What the conversation last sent the agent, when it was a message.
    #[cfg(test)]
    pub(super) fn agent_prompt(&self) -> Option<String> {
        match self.chat.as_ref()?.agent.as_ref()?.last_input.as_ref()? {
            TurnInput::Prompt(prompt) => Some(prompt.clone()),
            TurnInput::Command { .. } => None,
        }
    }

    /// Whether the conversation waits on a go-ahead for something that only
    /// reads: for live tests, which allow those and nothing else.
    #[cfg(test)]
    pub(super) fn waiting_on_a_read(&self) -> bool {
        let Some(agent) = self.chat.as_ref().and_then(|run| run.agent.as_ref()) else {
            return false;
        };
        matches!(
            agent.pending.as_ref(),
            Some(Pending::Permission(request)) if only_reads(request, &agent.directory)
        )
    }

    /// What the conversation waits on the user for, in a word.
    #[cfg(test)]
    pub(super) fn agent_waiting_on(&self) -> Option<&'static str> {
        let agent = self.chat.as_ref()?.agent.as_ref()?;
        if agent.sign_in.is_some() {
            return Some("sign-in");
        }
        Some(match agent.pending.as_ref()? {
            Pending::Edit(_) => "edit",
            Pending::Permission(_) => "permission",
            Pending::Question { .. } => "question",
        })
    }

    /// Send `input` to the agent, in the run's session once there is one,
    /// and follow what it does until it's done.
    pub(super) fn start_turn(
        &mut self,
        id: u64,
        input: TurnInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The agent reads the disk, so it must see what's on screen.
        if self.tab().dirty
            && let Some(path) = self.document().path.clone()
        {
            self.save_to(&path, window, cx);
        }
        // Learn the commands, so a reply can run one.
        if self.catalog.read(cx).catalog().is_none() {
            self.load_agent_catalog(None, cx);
        }
        // Read on this thread: the keys and settings live in the app.
        let pick = self.agent_run(id).and_then(|agent| agent.pick.clone());
        let choice = jig_agent::agent::choice_for(pick.as_ref(), cx);
        let Some(agent) = self.agent_run(id) else {
            return;
        };
        let session = agent.session.clone();
        let directory = agent.directory.clone();
        let title = agent.title.clone();
        let map_action = agent.map_action.clone();
        agent.proposed = None;
        agent.last_input = Some(input.clone());
        let permissions = jig_settings::get(cx).agent.permissions;
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
            started: agent.started,
            context: agent.context.clone(),
            step: agent.step.clone(),
        };

        let (sender, mut messages) = futures::channel::mpsc::unbounded();
        let turn = move || {
            let send = |message| {
                let _ = sender.unbounded_send(message);
            };
            let result = (|| {
                let choice = choice.map_err(anyhow::Error::msg)?;
                let session = match session {
                    Some(session) => session,
                    None => {
                        let backend = jig_agent::agent::connect(&choice)?;
                        let session =
                            backend.create(&directory, &title, &permissions, choice.acp_model())?;
                        if let Some(models) = backend.models() {
                            send(Message::Models(choice.key().to_string(), models));
                        }
                        send(Message::Started(session.clone()));
                        session
                    }
                };
                let (prompt, command) = match input {
                    TurnInput::Prompt(prompt) => (prompt, None),
                    TurnInput::Command { name, arguments } => (arguments, Some(name)),
                };
                let request = AgentRequest {
                    directory,
                    prompt,
                    model: choice.model(),
                    command,
                    permissions,
                };
                session.run(&request, &|event| send(Message::Event(event)))
            })();
            if let Err(error) = result {
                send(failure(error));
            }
        };
        // A turn blocks on the agent, so it has a thread of its own. Under
        // test it runs on the test scheduler instead, which fails a test
        // when another thread wakes it, as this one's messages would.
        #[cfg(not(test))]
        std::thread::spawn(turn);
        #[cfg(test)]
        cx.background_executor()
            .spawn(async move { turn() })
            .detach();
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
        if let Some(run) = self.chat.as_mut() {
            run.bubble = bubble;
            run._task = Some(task);
        }
        if let (Some(action), Some(pane)) = (map_action, self.map.as_ref()) {
            pane.view
                .update(cx, |view, cx| view.working_on_action(action, cx));
        }
        cx.notify();
    }

    /// Send `text` to the agent as the user's reply, between turns.
    pub(super) fn send_to_agent(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(run) = self.chat.as_mut() else {
            return;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut().filter(|agent| agent.finished) else {
            return;
        };
        agent.entries.push(ChatEntry::User(text.clone()));
        agent.scroll.scroll_to_bottom();
        self.start_turn(id, TurnInput::Prompt(text), window, cx);
    }

    /// Send to Agent (⇧⌘L, or the code's right-click menu): the selection,
    /// or the cursor's line, goes into the conversation's reply box as a
    /// reference to write about and send. With no conversation open, a new
    /// one opens empty, waiting for the message.
    pub(super) fn send_selection_to_agent(
        &mut self,
        _: &SendSelectionToAgent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home || self.map_showing().is_some() {
            return;
        }
        self.close_palette(window, cx);
        if self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_none()
        {
            let Some(id) = self.start_agent_run(window, cx) else {
                return;
            };
            let directory = self.agent_project(cx).expect("start_agent_run checked");
            let mut agent = self.new_agent_run(None, String::new(), directory, window, cx);
            agent.entries.clear();
            if let Some(run) = self.chat.as_mut().filter(|run| run.id == id) {
                run.agent = Some(agent);
            }
            let anchor = self.agent_anchor(window);
            if let Some(run) = self.chat.as_mut() {
                run.anchor = anchor;
            }
        }
        let Some(directory) = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map(|agent| agent.directory.clone())
        else {
            return;
        };
        let reference = self.selection_reference(&directory, cx);
        let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) else {
            return;
        };
        let id = SharedString::from(format!("code-{}", agent.references.len()));
        let name = reference.name();
        let token = InlineToken::new(id.clone(), format!("@{name}")).with_label(name);
        agent.references.push((id, reference));
        let input = agent.input.clone();
        // Its reply box shows once the agent's turn is over.
        let waiting = agent.finished;
        input.update(cx, |input, cx| {
            let before = input.value()[..input.cursor()].to_string();
            if before
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_whitespace())
            {
                input.insert(" ", window, cx);
            }
            if input.replace_with_token(token, window, cx).is_ok() {
                input.insert(" ", window, cx);
            }
        });
        if waiting {
            input.update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.show_note(
                "It's in the reply box, for when the agent's done.".into(),
                window,
                cx,
            );
        }
        cx.notify();
    }

    /// The selection, or the cursor's line, as code sent to the agent, its
    /// file named from the project `directory`.
    fn selection_reference(&self, directory: &Path, cx: &App) -> CodeReference {
        let text = self.editor().text(cx);
        let selection = self.editor().selection(cx);
        let range = if selection.is_empty() {
            super::commands::line_at(&text, selection.start)
        } else {
            selection
        };
        let line = |offset: usize| 1 + text[..offset].matches('\n').count();
        let file = match self.document().path.as_deref() {
            Some(path) => path
                .strip_prefix(directory)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned(),
            None => "untitled".into(),
        };
        CodeReference {
            file,
            language: self.editor().language(cx),
            lines: (line(range.start), line(range.end.max(range.start + 1) - 1)),
            code: text[range].to_string(),
        }
    }

    /// Enter in the reply box: the agent takes the reply as its next turn.
    fn reply_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.chat.as_mut() else {
            return;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut().filter(|agent| agent.finished) else {
            return;
        };
        let text = agent.input.read(cx).value().trim().to_string();
        if text.is_empty() {
            // Signed in in the terminal: Enter tries again.
            if agent.sign_in.take().is_some()
                && let Some(input) = agent.last_input.clone()
            {
                agent.entries.push(ChatEntry::Note("Trying again.".into()));
                self.focus_agent_or_editor(window, cx);
                self.start_turn(id, input, window, cx);
            }
            return;
        }
        agent.sign_in = None;
        let references = referred_to(&agent.input.read(cx).content(), &agent.references);
        agent.references.clear();
        // Opened empty by sending code over: what's asked first names it.
        let first = agent.session.is_none() && agent.entries.is_empty();
        if first {
            agent.title = text.clone();
        }
        agent
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        agent.entries.push(ChatEntry::User(text.clone()));
        agent.scroll.scroll_to_bottom();
        // Back to the code, where the agent's edits are reviewed, or the
        // Map, where a Map action's answer is.
        self.focus_code(window, cx);
        let input = self.slash_command(&text, cx).unwrap_or_else(|| {
            TurnInput::Prompt(jig_ai::agent::message(&text, &references, first))
        });
        self.start_turn(id, input, window, cx);
    }

    /// The sign-in option clicked: open the agent's login in a terminal, or
    /// ask the agent to sign in by itself and try again once it has.
    fn sign_in_with(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(run) = self.chat.as_mut() else {
            return;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut() else {
            return;
        };
        let Some(pending) = agent.sign_in.as_mut() else {
            return;
        };
        let Some(method) = pending.needed.methods.get(index).cloned() else {
            return;
        };
        match &method.terminal {
            Some(command) => match jig_agent::agent::open_in_terminal(command) {
                Ok(()) => {
                    pending.opened = true;
                    let input = agent.input.clone();
                    input.update(cx, |input, cx| input.focus(window, cx));
                }
                Err(error) => agent.entries.push(ChatEntry::Error(format!("{error:#}"))),
            },
            None => {
                agent.sign_in = None;
                agent
                    .entries
                    .push(ChatEntry::Note(format!("Signing in with {}…", method.name)));
                if let Some(input) = agent.last_input.clone() {
                    self.authenticate_then(id, method.id, input, window, cx);
                }
            }
        }
        cx.notify();
    }

    /// Ask the agent to sign in with `method` (it may open the browser),
    /// then send `input` again.
    fn authenticate_then(
        &mut self,
        id: u64,
        method: String,
        input: TurnInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pick = self.agent_run(id).and_then(|agent| agent.pick.clone());
        let choice = jig_agent::agent::choice_for(pick.as_ref(), cx);
        let task = cx.background_executor().spawn(async move {
            let backend = jig_agent::agent::connect(&choice.map_err(anyhow::Error::msg)?)?;
            if let jig_ai::agent::harness::Backend::Acp(connection) = backend {
                connection.authenticate(&method)?;
            }
            anyhow::Ok(())
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| match result {
                Ok(()) => this.start_turn(id, input, window, cx),
                Err(error) => {
                    this.on_agent_message(id, failure(error), window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// A reply like "/review main" runs that OpenCode command, when there
    /// is one by that name.
    fn slash_command(&self, text: &str, cx: &App) -> Option<TurnInput> {
        let typed = text.strip_prefix('/')?;
        let (name, arguments) = typed.split_once(' ').unwrap_or((typed, ""));
        self.catalog
            .read(cx)
            .catalog()?
            .commands
            .iter()
            .any(|command| command.name == name)
            .then(|| TurnInput::Command {
                name: name.to_string(),
                arguments: arguments.trim().to_string(),
            })
    }

    /// Answer the question on screen with option `choice`, or with what's
    /// in the reply box: an option's number or the user's own words.
    /// Returns false when no question is waiting.
    pub(super) fn answer_agent_question(
        &mut self,
        choice: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) else {
            return false;
        };
        let Some(Pending::Question { request, answers }) = agent.pending.as_mut() else {
            return false;
        };
        let question = &request.questions[answers.len()];
        let typed = agent.input.read(cx).value().trim().to_string();
        let answer = match choice {
            Some(index) => question.options.get(index).cloned(),
            None => typed
                .parse::<usize>()
                .ok()
                .and_then(|number| question.options.get(number.wrapping_sub(1)).cloned())
                .or_else(|| (!typed.is_empty()).then_some(typed)),
        };
        let Some(answer) = answer else {
            return true;
        };
        answers.push(answer);
        agent
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        if answers.len() < request.questions.len() {
            cx.notify();
            return true;
        }
        let Some(Pending::Question { request, answers }) = agent.pending.take() else {
            return true;
        };
        agent.entries.push(ChatEntry::User(answers.join(" · ")));
        agent.scroll.scroll_to_bottom();
        if let Some(session) = agent.session.clone() {
            cx.background_executor()
                .spawn(async move {
                    let _ = session.answer(&request.id, Some(&answers));
                })
                .detach();
        }
        let id = self.chat.as_ref().map_or(0, |run| run.id);
        self.focus_agent_or_editor(window, cx);
        self.show_next_request(id, window, cx);
        cx.notify();
        true
    }

    /// Enter in the reply box while the agent asks to save a plan as a
    /// task: saves it when nothing's typed, otherwise turns it down with
    /// what's typed for the agent to follow. Returns false when it isn't
    /// asking that.
    pub(super) fn answer_task_question(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(run) = self.chat.as_mut() else {
            return false;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut() else {
            return false;
        };
        if !matches!(&agent.pending, Some(Pending::Permission(request)) if request.task.is_some()) {
            return false;
        }
        let said = agent.input.read(cx).value().trim().to_string();
        if said.is_empty() {
            return self.settle_agent_request(true, window, cx);
        }
        let Some(Pending::Permission(request)) = agent.pending.take() else {
            return false;
        };
        agent
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        agent.entries.push(ChatEntry::User(said.clone()));
        agent.scroll.scroll_to_bottom();
        if let Some(session) = agent.session.clone() {
            let note = session.hears_notes().then(|| said.clone());
            if note.is_none() {
                agent.follow_up = Some(said);
            }
            cx.background_executor()
                .spawn(async move {
                    let _ = session.reply(&request.id, false, note.as_deref());
                })
                .detach();
        }
        self.focus_agent_or_editor(window, cx);
        self.show_next_request(id, window, cx);
        cx.notify();
        true
    }

    /// Allow or turn down what the agent asked to do (a command, the web, a
    /// subagent), or decline its question. Returns false when it isn't
    /// waiting on one.
    pub(super) fn settle_agent_request(
        &mut self,
        allow: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(run) = self.chat.as_mut() else {
            return false;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut() else {
            return false;
        };
        // Enter with words in the reply box answers the plan with them.
        let typed = !agent.input.read(cx).value().trim().is_empty();
        if allow
            && typed
            && matches!(&agent.pending, Some(Pending::Permission(request)) if request.task.is_some())
        {
            return self.answer_task_question(window, cx);
        }
        let session = agent.session.clone();
        match agent.pending.take() {
            Some(Pending::Permission(request)) => {
                if request.task.is_some() {
                    agent
                        .input
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    if !allow {
                        agent
                            .entries
                            .push(ChatEntry::Note("Not saved as a task.".into()));
                    }
                    self.focus_agent_or_editor(window, cx);
                } else if !allow {
                    agent
                        .entries
                        .push(ChatEntry::Note(format!("Not allowed: {}", request.detail)));
                }
                if let Some(session) = session {
                    cx.background_executor()
                        .spawn(async move {
                            let _ = session.reply(&request.id, allow, None);
                        })
                        .detach();
                }
            }
            // A question is answered in the reply box; Esc declines it.
            Some(Pending::Question { request, .. }) if !allow => {
                agent
                    .entries
                    .push(ChatEntry::Note("Declined to answer.".into()));
                agent
                    .input
                    .update(cx, |input, cx| input.set_value("", window, cx));
                if let Some(session) = session {
                    cx.background_executor()
                        .spawn(async move {
                            let _ = session.answer(&request.id, None);
                        })
                        .detach();
                }
                self.focus_agent_or_editor(window, cx);
            }
            other => {
                agent.pending = other;
                return false;
            }
        }
        if let Some(agent) = self.agent_run(id) {
            agent.scroll.scroll_to_bottom();
        }
        self.show_next_request(id, window, cx);
        cx.notify();
        true
    }

    /// ⌘↩ or Always allow: let the request through, and others like it
    /// from now on, as the agent remembers them or else for the rest of the
    /// conversation. Returns false when no request is waiting.
    pub(super) fn allow_agent_request_always(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(run) = self.chat.as_mut() else {
            return false;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut() else {
            return false;
        };
        let request = match agent.pending.take() {
            // Saving a plan has no "always": the next one is asked again.
            Some(Pending::Permission(request)) if request.task.is_some() => {
                agent.pending = Some(Pending::Permission(request));
                return self.settle_agent_request(true, window, cx);
            }
            Some(Pending::Permission(request)) => request,
            Some(edit @ Pending::Edit(_)) => {
                agent.pending = Some(edit);
                return self.accept_all_agent_edits(id, window, cx);
            }
            other => {
                agent.pending = other;
                return false;
            }
        };
        let agent_remembers = request.always.is_some();
        if !agent_remembers {
            agent.always_allowed.push(request.title.clone());
        }
        agent.entries.push(ChatEntry::Note(match &request.always {
            Some(always) => format!("{always}."),
            None => format!("{}: allowed for this conversation.", request.title),
        }));
        agent.scroll.scroll_to_bottom();
        if let Some(session) = agent.session.clone() {
            cx.background_executor()
                .spawn(async move {
                    let _ = if agent_remembers {
                        session.allow_always(&request.id)
                    } else {
                        session.reply(&request.id, true, None)
                    };
                })
                .detach();
        }
        self.show_next_request(id, window, cx);
        cx.notify();
        true
    }

    /// ⌘↩ on the agent's edit: accept it, and its later edits as they come.
    fn accept_all_agent_edits(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.settle_agent_edit(true, Some(window), cx) {
            return false;
        }
        if let Some(agent) = self.agent_run(id) {
            agent.accept_edits = true;
            agent.entries.push(ChatEntry::Note(
                "Edits: accepted for the rest of this conversation.".into(),
            ));
            agent.scroll.scroll_to_bottom();
        }
        self.show_next_request(id, window, cx);
        cx.notify();
        true
    }

    /// One of the request's buttons: 0 allows, 1 allows always, 2 denies.
    /// For an edit: 0 accepts it, 1 accepts it and the rest, 2 rejects it.
    fn decide_agent_request(&mut self, button: usize, window: &mut Window, cx: &mut Context<Self>) {
        let edit = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| matches!(agent.pending, Some(Pending::Edit(_))));
        match (edit, button) {
            (true, 0) => self.settle_agent_edit(true, Some(window), cx),
            (true, 2) => self.settle_agent_edit(false, Some(window), cx),
            (false, 0) => self.settle_agent_request(true, window, cx),
            (_, 1) => self.allow_agent_request_always(window, cx),
            _ => self.settle_agent_request(false, window, cx),
        };
        self.focus_agent_or_editor(window, cx);
    }

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

    /// The pin button: dock the conversation to the window's right edge, or
    /// float it at the code again.
    fn toggle_agent_chat_pin(&mut self, cx: &mut Context<Self>) {
        if let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) {
            agent.frame.pinned = !agent.frame.pinned;
            self.agent.dock = agent.frame.pinned.then_some(agent.frame.width);
            cx.notify();
        }
    }

    /// Mouse handlers for the whole window, active while the docked
    /// conversation's edge is dragged: it keeps the right edge and takes
    /// what's left of the mouse.
    pub(super) fn agent_dock_drag_handlers(&self, root: Div, cx: &Context<Self>) -> Div {
        const MIN: f32 = 320.;
        root.when(self.agent.resizing_dock, |root| {
            root.cursor_col_resize()
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                    if !event.dragging() {
                        this.agent.resizing_dock = false;
                    } else {
                        let viewport = window.viewport_size().width;
                        // The column's padding on both sides.
                        let width = viewport - event.position.x - px(16.);
                        let width = width.clamp(px(MIN), (viewport * 0.7).max(px(MIN)));
                        if let Some(agent) = this.chat.as_mut().and_then(|run| run.agent.as_mut()) {
                            agent.frame.width = width;
                            this.agent.dock = Some(width);
                        }
                    }
                    cx.notify();
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.agent.resizing_dock = false;
                        cx.notify();
                    }),
                )
        })
    }

    /// With no conversation selected, show the project's recent conversations.
    pub(super) fn show_agent_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_none()
        {
            if let Some(directory) = self.agent_project(cx)
                && self
                    .agent
                    .home_draft
                    .as_ref()
                    .is_none_or(|(project, _)| *project != directory)
            {
                let input = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .placeholder("Ask the agent about your project…")
                        .auto_grow(1, 8)
                        .soft_wrap(true)
                });
                self.agent.home_draft = Some((directory, input));
            }
            self.load_agent_catalog(None, cx);
        }
        self.focus_agent_mode(window, cx);
        cx.notify();
    }

    fn ensure_agent_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some()
        {
            return;
        }
        let Some(id) = self.start_agent_run(window, cx) else {
            return;
        };
        let directory = self.agent_project(cx).expect("start_agent_run checked");
        let mut agent = self.new_agent_run(None, String::new(), directory, window, cx);
        agent.entries.clear();
        agent.input.update(cx, |input, cx| {
            input.set_placeholder("Ask the agent about your project…", window, cx)
        });
        if let Some(run) = self.chat.as_mut().filter(|run| run.id == id) {
            run.agent = Some(agent);
        }
        self.focus_agent_mode(window, cx);
        cx.notify();
    }

    pub(super) fn focus_agent_mode(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(agent) = self.chat.as_ref().and_then(|run| run.agent.as_ref()) {
            if agent.loading
                || matches!(
                    &agent.pending,
                    Some(Pending::Permission(_)) | Some(Pending::Edit(_))
                )
            {
                agent.page_focus.focus(window, cx);
                return;
            }
            agent.input.update(cx, |input, cx| input.focus(window, cx));
        } else if let Some(input) = self.agent_home_input(cx) {
            input.update(cx, |input, cx| input.focus(window, cx));
        } else {
            self.home_focus.focus(window, cx);
        }
    }

    fn focus_agent_or_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == super::mode::Mode::Agent {
            self.focus_agent_mode(window, cx);
        } else {
            self.focus_editor(window, cx);
        }
    }

    pub(super) fn submit_agent_mode_prompt(
        &mut self,
        text: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if text.trim().is_empty() {
            return;
        }
        if self.agent_busy() {
            return self.show_note(
                "Wait for the agent, or stop its turn first.".into(),
                window,
                cx,
            );
        }
        self.ensure_agent_conversation(window, cx);
        if let Some(agent) = self.chat.as_ref().and_then(|run| run.agent.as_ref()) {
            agent
                .input
                .update(cx, |input, cx| input.set_value(text, window, cx));
        }
        self.reply_to_agent(window, cx);
    }

    fn agent_home_input(&self, cx: &App) -> Option<&Entity<TextareaState>> {
        let directory = self.agent_project(cx)?;
        self.agent
            .home_draft
            .as_ref()
            .filter(|(project, _)| *project == directory)
            .map(|(_, input)| input)
    }

    fn submit_agent_home_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(input) = self.agent_home_input(cx).cloned() else {
            return;
        };
        let text = input.read(cx).value().to_string();
        if text.trim().is_empty() {
            return;
        }
        self.submit_agent_mode_prompt(text, window, cx);
        if self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| agent.last_input.is_some())
        {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
    }

    fn agent_busy(&self) -> bool {
        self.chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| !agent.finished || agent.loading || agent.pending.is_some())
    }

    pub(super) fn new_agent_conversation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.agent_busy() || self.chat.as_ref().is_some_and(|run| run.preview.is_some()) {
            return self.show_note(
                "Finish or stop the current conversation first.".into(),
                window,
                cx,
            );
        }
        self.close_chat(window, cx);
        self.ensure_agent_conversation(window, cx);
    }

    pub(super) fn render_agent_mode(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let busy = self.agent_busy();
        v_flex()
            .id("agent-mode")
            // Keep the shared Enter/Tab/Escape review keys active when a request
            // owns focus and the message editor is not on screen.
            .key_context("Input")
            .debug_selector(|| "agent-mode".into())
            .track_focus(
                self.chat
                    .as_ref()
                    .and_then(|run| run.agent.as_ref())
                    .map_or(&self.home_focus, |agent| &agent.page_focus),
            )
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                h_flex()
                    .flex_wrap()
                    .px_4()
                    .py_2()
                    .gap_2()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("Agent"),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("agent-mode-choose")
                            .disabled(
                                self.chat
                                    .as_ref()
                                    .and_then(|run| run.agent.as_ref())
                                    .is_some_and(|agent| {
                                        !agent.entries.is_empty() || agent.loading
                                    }),
                            )
                            .label("Choose agent")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_command(&OpenCommand, window, cx);
                                if let Some(palette) = &this.palette {
                                    palette.view.update(cx, |palette, cx| {
                                        palette.start_picking(window, cx)
                                    });
                                }
                            })),
                    )
                    .child(
                        Button::new("agent-mode-history")
                            .label("Conversations")
                            .small()
                            .ghost()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_agent_conversations(&OpenAgentConversations, window, cx)
                            })),
                    )
                    .when(
                        self.chat
                            .as_ref()
                            .and_then(|run| run.agent.as_ref())
                            .is_some(),
                        |this| {
                            this.child(
                                Button::new("agent-mode-new")
                                    .label("New conversation")
                                    .small()
                                    .ghost()
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.new_agent_conversation(window, cx)
                                    })),
                            )
                        },
                    )
                    .when(busy, |this| {
                        this.child(
                            Button::new("agent-mode-stop")
                                .label("Stop")
                                .small()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.stop_agent_turn(window, cx);
                                })),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(
                        div().h_full().w_full().max_w(px(960.)).min_w_0().child(
                            self.render_agent_chat(cx)
                                .unwrap_or_else(|| self.render_recent_conversations(cx)),
                        ),
                    ),
            )
            .into_any_element()
    }

    fn render_recent_conversations(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let directory = self.agent_project(cx);
        let agent = self.picked_agent(cx);
        let catalog = self.catalog.read(cx);
        let mut recent: Vec<_> = directory
            .as_deref()
            .and_then(|directory| catalog.for_project(directory, &agent.key))
            .into_iter()
            .flat_map(|catalog| &catalog.conversations)
            .collect();
        recent.sort_by_key(|conversation| std::cmp::Reverse(conversation.updated));
        let empty = recent.is_empty();
        let now = SystemTime::now();
        v_flex()
            .id("agent-recents")
            .debug_selector(|| "agent-recents".into())
            .size_full()
            .p_4()
            .gap_3()
            .child(div().text_size(px(20.)).child("Recent conversations"))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "{} · {}",
                        agent.name,
                        directory
                            .as_ref()
                            .and_then(|path| path.file_name())
                            .map(|name| name.to_string_lossy())
                            .unwrap_or_default()
                    )),
            )
            .when(catalog.loading(), |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("Loading conversations…"),
                )
            })
            .when_some(catalog.error(), |this, error| {
                this.child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(error.clone()),
                        )
                        .child(
                            Button::new("agent-recents-retry")
                                .label("Try again")
                                .small()
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.load_agent_catalog(None, cx)),
                                ),
                        ),
                )
            })
            .when(
                empty && !catalog.loading() && catalog.error().is_none(),
                |this| {
                    this.child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child("No conversations yet. Type a message below to start one."),
                    )
                },
            )
            .child(
                v_flex()
                    .id("agent-recents-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.agent.recent_scroll)
                    .gap_1()
                    .children(recent.into_iter().take(30).enumerate().map(
                        |(index, conversation)| {
                            let id = conversation.id.clone();
                            let title = conversation_title(conversation);
                            div()
                                .debug_selector(move || format!("agent-recent-{index}"))
                                .child(
                                    Button::new(("agent-recent", index))
                                        .label(format!(
                                            "{title} · {}",
                                            ago(now, conversation.updated)
                                        ))
                                        .w_full()
                                        .ghost()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.resume_agent(id.clone(), window, cx)
                                        })),
                                )
                        },
                    )),
            )
            .when_some(self.agent_home_input(cx), |this, input| {
                this.child(
                    v_flex()
                        .flex_none()
                        .gap_2()
                        .child(jig_commands::chat::message_input(
                            input,
                            cx.listener(|this, _, window, cx| {
                                this.submit_agent_home_message(window, cx)
                            }),
                            cx,
                        ))
                        .child(jig_commands::surface::hint(
                            "enter to send · shift-enter for a new line",
                            cx,
                        )),
                )
            })
            .into_any_element()
    }

    #[cfg(test)]
    pub(super) fn test_recent_conversations(
        &mut self,
        conversations: Vec<jig_ai::agent::Conversation>,
        cx: &mut Context<Self>,
    ) {
        let catalog = AgentCatalog {
            agent: self.picked_agent(cx).key.clone(),
            directory: self.agent_project(cx).unwrap(),
            conversations,
            commands: Vec::new(),
        };
        self.catalog
            .update(cx, |entity, cx| entity.set(catalog, cx));
        cx.notify();
    }

    /// The conversation docked to the window's right edge, while pinned.
    pub(super) fn render_pinned_agent_chat(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let agent = self.chat.as_ref()?.agent.as_ref()?;
        if self.mode == super::mode::Mode::Agent || !agent.frame.pinned {
            return None;
        }
        // Opaque like the code beside it: the window behind is translucent.
        // Its left edge resizes it, like the sidebar's right edge.
        let edge = div()
            .id("agent-dock-resize")
            .debug_selector(|| "agent-dock-resize".into())
            .absolute()
            .top_0()
            .bottom_0()
            .left(px(-3.))
            .w(px(6.))
            .cursor_col_resize()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.agent.resizing_dock = true;
                    cx.stop_propagation();
                }),
            );
        Some(
            div()
                .relative()
                .flex_none()
                .h_full()
                .p_2()
                .bg(jig_kit::theme::editor_surface(cx))
                .border_l_1()
                .border_color(cx.theme().title_bar_border)
                .children(self.render_agent_chat(cx))
                .child(edge)
                .into_any_element(),
        )
    }

    /// The conversation offers ways to sign in to the agent.
    fn signing_in(&self) -> bool {
        self.chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| agent.sign_in.is_some())
    }

    /// How much of the window's right a docked conversation takes.
    pub(super) fn docked_chat_width(&self) -> Pixels {
        if self.mode == super::mode::Mode::Agent {
            return px(0.);
        }
        self.chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .filter(|agent| agent.frame.pinned)
            .map_or(px(0.), |agent| agent.frame.width)
    }

    /// The conversation floating at the code, unless it's pinned.
    pub(super) fn floating_agent_chat_pinned(&self) -> bool {
        if self.mode == super::mode::Mode::Agent {
            return true;
        }
        self.chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| agent.frame.pinned)
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

    /// The reply box's text, and the code its tokens name.
    #[cfg(test)]
    pub(super) fn agent_reply(&self, cx: &App) -> (String, Vec<CodeReference>) {
        let Some(agent) = self.chat.as_ref().and_then(|run| run.agent.as_ref()) else {
            return Default::default();
        };
        let content = agent.input.read(cx).content();
        (
            content.text().to_string(),
            referred_to(&content, &agent.references),
        )
    }

    #[cfg(test)]
    pub(super) fn agent_entries(&self) -> Vec<ChatEntry> {
        self.chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map(|agent| agent.entries.clone())
            .unwrap_or_default()
    }

    /// Where the conversation opens: centred in the window's right half,
    /// top to bottom at the height it grows to, whatever code it's about.
    pub(super) fn agent_anchor(&self, window: &Window) -> Point<Pixels> {
        let frame = self
            .chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map_or_else(Default::default, |agent| agent.frame.clone());
        self.right_centre(frame.width, frame.full_height(), window)
    }

    /// Pressed on the conversation's header: a drag may follow.
    fn grab_agent_chat(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(run) = self.chat.as_mut() else {
            return;
        };
        let Some(agent) = run.agent.as_mut() else {
            return;
        };
        // From where it's drawn, which may differ from the anchor when it
        // was pushed in to fit the window.
        let origin = agent.frame.bounds.get().origin;
        agent.grab = Some(event.position - origin);
        run.anchor = origin;
        cx.stop_propagation();
    }

    /// Pressed on the conversation's corner grip: a resize may follow.
    fn grab_agent_chat_corner(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some(run) = self.chat.as_mut() else {
            return;
        };
        let Some(agent) = run.agent.as_mut() else {
            return;
        };
        let height = agent.frame.transcript_bounds.get().size.height;
        agent.resize = Some((event.position, size(agent.frame.width, height)));
        // It grows from where it's drawn.
        run.anchor = agent.frame.bounds.get().origin;
        cx.stop_propagation();
    }

    /// The conversation's corner follows the mouse while it's resized.
    pub(super) fn resize_agent_chat(
        &mut self,
        event: &DragMoveEvent<ChatResize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        const MIN: Size<Pixels> = Size {
            width: px(320.),
            height: px(60.),
        };
        let viewport = window.viewport_size();
        let Some(run) = self.chat.as_mut() else {
            return;
        };
        let Some(agent) = run.agent.as_mut() else {
            return;
        };
        let Some((start, from)) = agent.resize else {
            return;
        };
        // The resize cursor stays on wherever the mouse goes.
        cx.set_active_drag_cursor_style(CursorStyle::ResizeUpLeftDownRight, window);
        let by = event.event.position - start;
        // Room for the header and reply box below the transcript.
        let max = size(viewport.width - px(16.), viewport.height - px(180.));
        agent.frame.width = (from.width + by.x).clamp(MIN.width, max.width.max(MIN.width));
        agent.frame.transcript_height =
            Some((from.height + by.y).clamp(MIN.height, max.height.max(MIN.height)));
        cx.notify();
    }

    /// The conversation follows the mouse while its header is dragged.
    pub(super) fn drag_agent_chat(
        &mut self,
        event: &DragMoveEvent<ChatDrag>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.set_active_drag_cursor_style(CursorStyle::ClosedHand, window);
        let Some(run) = self.chat.as_mut() else {
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
        let run = self.chat.as_ref()?;
        let agent = run.agent.as_ref()?;
        let status = match (&agent.pending, &run.bubble) {
            _ if agent.loading => ChatStatus::Loading,
            _ if let Some(sign_in) = agent.sign_in.as_ref() => ChatStatus::SignIn {
                options: sign_in
                    .needed
                    .methods
                    .iter()
                    .map(|method| {
                        if method.description.is_empty() {
                            method.name.clone()
                        } else {
                            format!("{}: {}", method.name, method.description)
                        }
                    })
                    .collect(),
                opened: sign_in.opened,
            },
            (
                Some(Pending::Permission(PermissionRequest {
                    task: Some(task), ..
                })),
                _,
            ) => ChatStatus::SaveTask {
                title: task.title.clone(),
                todos: task.todos.clone(),
            },
            (Some(Pending::Permission(request)), _) => ChatStatus::Asking {
                title: request.title.clone(),
                detail: request.detail.clone(),
                always: request
                    .always
                    .clone()
                    .unwrap_or_else(|| "Allow in this conversation".into()),
            },
            (Some(Pending::Question { request, answers }), _) => {
                let question = &request.questions[answers.len()];
                ChatStatus::Question {
                    question: question.question.clone(),
                    options: question.options.clone(),
                    position: (answers.len() + 1, request.questions.len()),
                }
            }
            (_, bubble) => match bubble {
                Bubble::Running { started, step, .. } => ChatStatus::Working {
                    started: *started,
                    step: step.clone(),
                },
                Bubble::Preview { message, removed } => ChatStatus::Reviewing {
                    path: message.clone(),
                    removed: removed.clone(),
                },
                _ => ChatStatus::Waiting,
            },
        };
        Some(
            Conversation::new(
                format!("{} · {}", agent.label, agent.project),
                linked_entries(&agent.entries, agent.directory()),
                status,
                agent.input.clone(),
                agent.scroll.clone(),
                agent.frame.clone(),
            )
            .full_page(self.mode == super::mode::Mode::Agent)
            .on_submit(cx.listener(|this, _, window, cx| {
                if !this.answer_agent_question(None, window, cx)
                    && !this.answer_task_question(window, cx)
                {
                    this.reply_to_agent(window, cx);
                }
            }))
            .on_grab(cx.listener(|this, event, _, cx| this.grab_agent_chat(event, cx)))
            .on_resize_grab(
                cx.listener(|this, event, _, cx| this.grab_agent_chat_corner(event, cx)),
            )
            .on_pin(cx.listener(|this, _, _, cx| this.toggle_agent_chat_pin(cx)))
            .models(
                self.conversation_models(cx).1,
                cx.listener(|this, _, _, cx| this.toggle_conversation_models(cx)),
                cx.listener(|this, index: &usize, _, cx| this.pick_conversation_model(*index, cx)),
            )
            .on_show_edit(cx.listener(|this, _, window, cx| {
                this.show_agent_edit(window, cx);
            }))
            .on_history(cx.listener(|this, _, window, cx| {
                this.open_agent_conversations(&OpenAgentConversations, window, cx)
            }))
            .on_decide(cx.listener(|this, button: &usize, window, cx| {
                this.decide_agent_request(*button, window, cx)
            }))
            .on_option(cx.listener(|this, index: &usize, window, cx| {
                if this.signing_in() {
                    this.sign_in_with(*index, window, cx);
                } else {
                    this.answer_agent_question(Some(*index), window, cx);
                }
            }))
            .on_drawing(
                cx.listener(|this, (at, action): &(usize, DrawingAction), window, cx| {
                    this.act_on_drawing(*at, *action, window, cx)
                }),
            )
            .on_link({
                let this = cx.weak_entity();
                std::sync::Arc::new(
                    move |link: &SharedString, window: &mut Window, cx: &mut App| {
                        let link = link.to_string();
                        this.update(cx, |this, cx| this.open_agent_link(&link, window, cx))
                            .ok();
                    },
                )
            })
            .into_any_element(),
        )
    }

    pub(super) fn agent_run(&mut self, id: u64) -> Option<&mut AgentRun> {
        self.chat
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
            Message::Models(key, models) => jig_agent::agents::remember_models(&key, models),
            Message::Resumed(session, turns) => {
                agent.session = Some(session);
                agent.loading = false;
                (agent.entries, agent.drawings) = super::map_agent::transcript(turns);
                agent.scroll.scroll_to_bottom();
                let input = agent.input.clone();
                input.update(cx, |input, cx| input.focus(window, cx));
                self.sync_map_drawings(cx);
                cx.notify();
                return false;
            }
            Message::Event(AgentEvent::Step(step)) => agent.step.set(step),
            Message::Event(AgentEvent::Edit(edit)) => agent.queue.push_back(Request::Edit(edit)),
            Message::Event(AgentEvent::Permission(request)) => {
                agent.queue.push_back(Request::Permission(request))
            }
            Message::Event(AgentEvent::Question(request)) => {
                agent.queue.push_back(Request::Question(request))
            }
            Message::Event(AgentEvent::Drew { tool, .. }) if tool == jig_ai::map::PROJECT => {
                agent
                    .entries
                    .push(ChatEntry::Did("Read Jig's outline of the project".into()));
                cx.notify();
            }
            Message::Event(AgentEvent::Drew { tool, input })
                if tool == jig_ai::map::PROPOSE_DOMAINS =>
            {
                agent
                    .entries
                    .push(ChatEntry::Did("Proposed domains".into()));
                if agent.map_action.is_some() {
                    agent.proposed = Some(input);
                }
                cx.notify();
            }
            Message::Event(AgentEvent::Drew { tool, input }) => {
                self.agent_drew(id, &tool, &input, cx);
            }
            Message::Event(AgentEvent::Commands(commands)) => {
                let directory = agent.directory.clone();
                self.learn_commands(directory, commands, cx);
            }
            Message::Event(AgentEvent::Did(text)) => {
                // An edit that skipped review stays in sight, not folded
                // away with the tool calls.
                agent.entries.push(if text.ends_with("without asking") {
                    ChatEntry::Note(text)
                } else {
                    ChatEntry::Did(text)
                });
                agent.scroll.scroll_to_bottom();
                cx.notify();
            }
            Message::Event(AgentEvent::Subagent { id, description }) => {
                let (named, _, _) = agent.subagent(&id);
                *named = description;
                agent.step.set("Working with a subagent".into());
                cx.notify();
            }
            Message::Event(AgentEvent::SubagentDid { id, text }) => {
                agent.subagent(&id).1.push(text);
                cx.notify();
            }
            Message::Event(AgentEvent::SubagentDone { id, report }) => {
                *agent.subagent(&id).2 = Some(report);
                cx.notify();
            }
            Message::Event(AgentEvent::Done(text)) => {
                agent.finished = true;
                let action = agent.map_action.clone();
                let proposed = agent.proposed.take();
                let follow_up = agent.follow_up.take();
                self.finish_agent(text.clone(), window, cx);
                if let Some(action) = action {
                    self.answer_map_action(action, &text, proposed.as_ref(), window, cx);
                }
                // Already in the transcript, from when the user said it.
                if let Some(said) = follow_up {
                    self.start_turn(id, TurnInput::Prompt(said), window, cx);
                }
                return false;
            }
            Message::SignIn(needed) => {
                agent.finished = true;
                agent.follow_up = None;
                agent.loading = false;
                let said = needed.to_string();
                agent.sign_in = Some(PendingSignIn {
                    needed,
                    opened: false,
                });
                let mapped = agent.map_action.is_some();
                if mapped {
                    self.end_map_action(cx);
                }
                self.end_turn(ChatEntry::Note(said), window, cx);
                return false;
            }
            Message::Failed(error) => {
                agent.finished = true;
                agent.follow_up = None;
                agent.loading = false;
                if agent.map_action.is_some() {
                    self.end_map_action(cx);
                }
                let error = super::commands::shorten(&error);
                self.end_turn(ChatEntry::Error(error), window, cx);
                return false;
            }
        }
        // A request may have queued up behind one the user just settled.
        self.show_next_request(id, window, cx);
        true
    }

    /// What the agent just said it offers, for the palette's "/" and for
    /// replies.
    fn learn_commands(
        &mut self,
        directory: PathBuf,
        commands: Vec<jig_ai::agent::AgentCommand>,
        cx: &mut Context<Self>,
    ) {
        let agent = self.picked_agent(cx).key.clone();
        let commands = commands
            .into_iter()
            .map(|command| AgentCommandInfo {
                name: command.name,
                description: command.description,
                skill: command.skill,
            })
            .collect();
        self.catalog.update(cx, |catalog, cx| {
            catalog.learn_commands(directory, agent, commands, cx)
        });
    }

    /// Put the next queued request on screen, unless one is already there.
    fn show_next_request(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent) = self.agent_run(id) else {
            return;
        };
        if agent.pending.is_some() {
            return;
        }
        match agent.queue.pop_front() {
            None => {}
            Some(Request::Edit(edit)) if agent.accept_edits => {
                match self.write_agent_edit(&edit, window, cx) {
                    Ok(true) => self.show_next_request(id, window, cx),
                    // Unsaved changes in its file: left to `show_edit`, which
                    // tells the agent to leave it alone.
                    Ok(false) => {
                        if let Err(error) = self.show_edit(&edit, window, cx) {
                            self.answer_edit(&edit.id, false, Some(&error), cx);
                            self.show_next_request(id, window, cx);
                        }
                    }
                    Err(error) => {
                        self.answer_edit(&edit.id, false, Some(&error), cx);
                        self.show_next_request(id, window, cx);
                    }
                }
            }
            Some(Request::Edit(edit)) => {
                if let Err(error) = self.show_edit(&edit, window, cx) {
                    // Tell the agent, so it can try again.
                    self.answer_edit(&edit.id, false, Some(&error), cx);
                    self.show_next_request(id, window, cx);
                }
            }
            // Asked in the conversation, answered with the review keys in
            // the code.
            // Allowed always earlier in this conversation, a shell command
            // that only reads the project, or, for a Map action, anything
            // that only reads.
            Some(Request::Permission(request))
                if agent.always_allowed.contains(&request.title)
                    || (request.kind == Asking::Command
                        && command_only_reads(&request.detail, &agent.directory))
                    || (agent.map_action.is_some() && only_reads(&request, &agent.directory)) =>
            {
                if let Some(session) = agent.session.clone() {
                    cx.background_executor()
                        .spawn(async move {
                            let _ = session.reply(&request.id, true, None);
                        })
                        .detach();
                }
                self.show_next_request(id, window, cx);
            }
            // Saved with Enter, or answered in the reply box.
            Some(Request::Permission(request)) if request.task.is_some() => {
                agent.pending = Some(Pending::Permission(request));
                agent.scroll.scroll_to_bottom();
                let input = agent.input.clone();
                input.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
            }
            Some(Request::Permission(request)) => {
                agent.pending = Some(Pending::Permission(request));
                agent.scroll.scroll_to_bottom();
                self.focus_agent_or_editor(window, cx);
                cx.notify();
            }
            // Answered in the reply box.
            Some(Request::Question(request)) if !request.questions.is_empty() => {
                agent.pending = Some(Pending::Question {
                    request,
                    answers: Vec::new(),
                });
                agent.scroll.scroll_to_bottom();
                let input = agent.input.clone();
                input.update(cx, |input, cx| input.focus(window, cx));
                cx.notify();
            }
            Some(Request::Question(_)) => self.show_next_request(id, window, cx),
        }
    }

    /// Accept the agent's edit without showing it, its edits having been
    /// accepted for the conversation: written to disk (by Jig or by the
    /// agent once told), its open tab caught up, and listed. False, with
    /// nothing done, when its open tab has unsaved changes.
    fn write_agent_edit(
        &mut self,
        edit: &EditRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, String> {
        let tab = self.tab_for(&edit.path);
        if tab.is_some_and(|ix| self.tabs[ix].dirty) {
            return Ok(false);
        }
        let created = !edit.path.exists();
        let disk = std::fs::read_to_string(&edit.path).unwrap_or_default();
        let new = edit.change.apply(&disk).map_err(|error| {
            format!("Jig couldn't apply that edit: {error:#}. Read the file again.")
        })?;
        if edit.change.written_by_jig() {
            std::fs::write(&edit.path, &new)
                .map_err(|error| format!("Jig couldn't write the file: {error}"))?;
            if let Some(ix) = tab {
                self.catch_up_with_disk(ix, window, cx);
            }
        }
        if created {
            self.refresh_tree(cx);
        }
        self.answer_edit(&edit.id, true, None, cx);
        let path = self.relative(&edit.path);
        let diff = jig_kit::diff::edit_lines(&disk, &new);
        if let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) {
            agent.entries.push(ChatEntry::Edit {
                path,
                accepted: true,
                diff,
            });
            // Written by the agent: caught up with when the turn ends.
            agent.touched.push(edit.path.clone());
            agent.scroll.scroll_to_bottom();
        }
        cx.notify();
        Ok(true)
    }

    fn show_edit(
        &mut self,
        edit: &EditRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        let created = !edit.path.exists();
        let disk = std::fs::read_to_string(&edit.path).unwrap_or_default();
        let new = edit.change.apply(&disk).map_err(|error| {
            format!("Jig couldn't apply that edit: {error:#}. Read the file again.")
        })?;
        if created {
            std::fs::write(&edit.path, "").map_err(|error| error.to_string())?;
            self.refresh_tree(cx);
        }

        let from_agent = self.mode == super::mode::Mode::Agent;
        if from_agent {
            self.remember_location(cx);
        }
        // Show the file, keeping the run alive across the tab switch.
        let on_screen = self
            .document()
            .path
            .as_deref()
            .is_some_and(|open| canonical(open) == canonical(&edit.path));
        if !on_screen {
            let run = self.chat.take();
            self.open_file(&edit.path, window, cx);
            self.chat = run;
        }
        if self.tab().dirty {
            return Err("The user has unsaved changes in this file; leave it alone.".into());
        }
        self.catch_up_with_disk(self.active, window, cx);

        let old = self.editor().text(cx);
        let preview = jig_kit::diff::preview_edit(&old, &new, 0);
        let relative = self.relative(&edit.path);
        let Some(run) = self.chat.as_mut() else {
            return Ok(());
        };
        run.preview = Some(Preview {
            range: preview.range.clone(),
        });
        run.bubble = Bubble::Preview {
            message: relative,
            removed: preview.removed,
        };
        if let Some(agent) = run.agent.as_mut() {
            agent.pending = Some(Pending::Edit(PendingEdit {
                id: edit.id.clone(),
                path: edit.path.clone(),
                created,
                jig_writes: edit.change.written_by_jig(),
                before: old.clone(),
            }));
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
        if let Some(preview) = self.chat.as_mut().and_then(|run| run.preview.as_mut()) {
            preview.range = range.clone();
        }
        if from_agent {
            self.focus_editor(window, cx);
        }
        // The bubble follows the change once it's scrolled into view.
        cx.on_next_frame(window, move |this, window, cx| {
            let anchor = this.agent_anchor(window);
            if let Some(run) = this
                .chat
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
        let mut window = window;
        // The edit may wait in another tab: settle it there. Rejecting
        // shows it, since its undo needs the window; accepting (e.g. as the
        // palette opens) doesn't move the user.
        let shown = self.active;
        let elsewhere = self
            .agent_edit_elsewhere()
            .and_then(|path| self.tab_for(&path));
        let mut quietly = false;
        match (elsewhere, window.as_deref_mut()) {
            (Some(ix), Some(window)) => self.activate(ix, window, cx),
            (Some(ix), None) => {
                self.active = ix;
                quietly = true;
            }
            _ => {}
        }
        let settled = self.settle_agent_edit_here(accept, window, cx);
        if quietly && shown < self.tabs.len() {
            self.active = shown;
        }
        settled
    }

    fn settle_agent_edit_here(
        &mut self,
        accept: bool,
        mut window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        // The edit as proposed, still in the buffer, for its diff.
        let proposed = self.editor().text(cx);
        let Some(run) = self.chat.as_mut() else {
            return false;
        };
        let id = run.id;
        let Some(agent) = run.agent.as_mut() else {
            return false;
        };
        let pending = match agent.pending.take() {
            Some(Pending::Edit(pending)) => pending,
            other => {
                agent.pending = other;
                return false;
            }
        };
        let preview = run.preview.take();
        if let Some(agent) = run.agent.as_mut() {
            let path = match &run.bubble {
                Bubble::Preview { message, .. } => message.clone(),
                _ => pending.path.to_string_lossy().into_owned(),
            };
            agent.entries.push(ChatEntry::Edit {
                path,
                accepted: accept,
                diff: jig_kit::diff::edit_lines(&pending.before, &proposed),
            });
            agent.scroll.scroll_to_bottom();
            run.bubble = Bubble::Running {
                label: agent.label.clone(),
                started: agent.started,
                context: agent.context.clone(),
                step: agent.step.clone(),
            };
            agent.step.set("Working".into());
            if accept {
                agent.touched.push(pending.path.clone());
            }
        }
        self.editor().clear_highlights(cx);
        self.editor().set_readonly(false, cx);
        let mut note = None;
        if accept {
            // The agent writes the same text to disk, or leaves it to Jig.
            let text = self.editor().text(cx);
            if pending.jig_writes
                && let Err(error) = std::fs::write(&pending.path, &text)
            {
                note = Some(format!("Jig couldn't write the file: {error}"));
            }
            let tab = self.tab_mut();
            tab.document.saved_text = text;
            tab.dirty = false;
        } else if let Some(window) = window.as_deref_mut() {
            // Put the old text back over the new, rather than undo: undo
            // finds nothing when no key has gone through the editor since
            // the edit, as when Reject is clicked.
            if let Some(preview) = preview {
                let now = self.editor().text(cx);
                let start = preview.range.start;
                let old = (pending.before.len() + preview.range.len())
                    .checked_sub(now.len())
                    .and_then(|old_len| pending.before.get(start..start + old_len));
                if let Some(old) = old {
                    self.editor().apply_edit(preview.range, old, window, cx);
                    self.editor().select(start..start, cx);
                }
            }
            if pending.created {
                let _ = std::fs::remove_file(&pending.path);
                let run = self.chat.take();
                let tab = self.tab().id();
                self.remove_tab(tab, window, cx);
                self.chat = run;
                self.refresh_tree(cx);
            }
        }
        let written = note.is_none();
        self.answer_edit(&pending.id, accept && written, note.as_deref(), cx);
        // The agent may have asked for more at once, e.g. several edits, and
        // sends nothing else until those are answered too: show the next.
        if let Some(window) = window {
            self.show_next_request(id, window, cx);
        }
        cx.notify();
        true
    }

    fn answer_edit(&self, edit_id: &str, accept: bool, note: Option<&str>, cx: &mut Context<Self>) {
        let session = self
            .chat
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
        if let Some(run) = self.chat.as_mut() {
            run.bubble = Bubble::AgentDone(text.clone());
        }
        self.end_turn(ChatEntry::Agent(text), window, cx);
    }

    /// Close the agent's turn with `entry` and open the reply box.
    fn end_turn(&mut self, entry: ChatEntry, window: &mut Window, cx: &mut Context<Self>) {
        let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) else {
            return;
        };
        let touched = std::mem::take(&mut agent.touched);
        agent.entries.push(entry);
        agent.scroll.scroll_to_bottom();
        let input = agent.input.clone();
        if let Some(run) = self.chat.as_mut()
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
        let (range, replacement) = jig_kit::diff::changed_range(&text, &disk);
        tab.editor.apply_edit(range, replacement, window, cx);
        tab.document.saved_text = disk;
        tab.dirty = false;
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

fn conversation_title(conversation: &jig_ai::agent::Conversation) -> String {
    conversation
        .title
        .clone()
        .unwrap_or_else(|| "Earlier conversation".into())
}

/// Hand the catalog to the palette, if it's still open.
fn show_catalog(catalog: &AgentCatalog, palette: &Entity<CommandPalette>, cx: &mut App) {
    let now = SystemTime::now();
    let conversations = catalog
        .conversations
        .iter()
        .map(|conversation| PastConversation {
            id: conversation.id.clone(),
            title: conversation_title(conversation),
            when: ago(now, conversation.updated),
        })
        .collect();
    let commands = catalog.commands.clone();
    palette.update(cx, |palette, cx| {
        palette.set_agent_catalog(conversations, commands, cx)
    });
}

/// How long ago `then` was, roughly: "just now", "5 min ago", "yesterday".
fn ago(now: SystemTime, then: SystemTime) -> String {
    let seconds = now.duration_since(then).unwrap_or_default().as_secs();
    let (minutes, hours, days) = (seconds / 60, seconds / 3600, seconds / 86_400);
    match () {
        _ if minutes < 1 => "just now".into(),
        _ if hours < 1 => format!("{minutes} min ago"),
        _ if days < 1 => format!("{hours} h ago"),
        _ if days < 2 => "yesterday".into(),
        _ if days < 30 => format!("{days} days ago"),
        _ => format!("{} months ago", days / 30),
    }
}

/// Where the choice for `agent` and `model` is in `choices`, adding it
/// (under `group`, as `name`) when it isn't listed, e.g. while models load.
fn position_or_push(
    choices: &mut Vec<ModelChoice>,
    agent: Option<&str>,
    model: Option<&str>,
    group: &str,
    name: &str,
) -> usize {
    let same_agent = |choice: &ModelChoice| choice.agent.as_deref() == agent;
    if let Some(at) = choices
        .iter()
        .position(|choice| same_agent(choice) && choice.model.as_deref() == model)
    {
        return at;
    }
    // An agent on its default: the first of its own.
    if model.is_none()
        && let Some(at) = choices.iter().position(same_agent)
    {
        return at;
    }
    choices.push(ModelChoice {
        agent: agent.map(str::to_string),
        group: group.to_string(),
        name: name.to_string(),
        model: model.map(str::to_string),
        label: format!("{group} · {name}"),
    });
    choices.len() - 1
}

/// The transcript with the files the agent names but didn't link made
/// links, so every file it mentions opens with a click.
fn linked_entries(entries: &[ChatEntry], project: &Path) -> Vec<ChatEntry> {
    let file = |written: &str| {
        let path = Path::new(written);
        let relative = path.strip_prefix(project).unwrap_or(path);
        let relative = relative.strip_prefix("./").unwrap_or(relative);
        (relative.is_relative() && project.join(relative).is_file())
            .then(|| relative.to_string_lossy().replace(' ', "%20"))
    };
    entries
        .iter()
        .map(|entry| match entry {
            ChatEntry::Agent(text) => ChatEntry::Agent(link_files(text, file)),
            ChatEntry::Subagent {
                id,
                description,
                calls,
                report,
            } => ChatEntry::Subagent {
                id: id.clone(),
                description: description.clone(),
                calls: calls.clone(),
                report: report.as_deref().map(|report| link_files(report, file)),
            },
            entry => entry.clone(),
        })
        .collect()
}

/// The code a reply's tokens name, each once, in the order they're named.
fn referred_to(
    content: &InputContent,
    references: &[(SharedString, CodeReference)],
) -> Vec<CodeReference> {
    let mut named: Vec<&SharedString> = Vec::new();
    content
        .tokens()
        .iter()
        .map(|span| span.token().id())
        .filter_map(|id| {
            if named.contains(&id) {
                return None;
            }
            named.push(id);
            references
                .iter()
                .find(|(known, _)| known == id)
                .map(|(_, reference)| reference.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    #[test]
    fn says_roughly_how_long_ago() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000);
        let ago = |seconds| super::ago(now, now - Duration::from_secs(seconds));
        assert_eq!(ago(20), "just now");
        assert_eq!(ago(300), "5 min ago");
        assert_eq!(ago(7_200), "2 h ago");
        assert_eq!(ago(100_000), "yesterday");
        assert_eq!(ago(5 * 86_400), "5 days ago");
    }
}
