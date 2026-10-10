//! Agent mode: the request goes to a coding agent (OpenCode), which may
//! read and edit the whole project. Every edit it wants to make opens in the
//! editor as a normal preview, in its file's tab, and only goes through when
//! the user accepts it; shell commands, the web and subagents wait for a
//! go-ahead too. The run is a conversation: once the agent answers, the
//! user can reply, and the agent carries on in the same session. Earlier
//! conversations are listed in the palette's Agent mode and can be picked
//! up again.

use std::collections::VecDeque;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use futures::StreamExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_ai::PromptRequest;
use jig_ai::acp::SignInNeeded;
use jig_ai::agent::{
    AgentEvent, AgentRequest, EditRequest, PermissionRequest, QuestionRequest, Turn,
};
use jig_ai::harness::Session;
use jig_commands::{
    AgentCommandInfo, Bubble, ChatDrag, ChatEntry, ChatFrame, ChatResize, ChatStatus,
    CommandPalette, Conversation, LiveStep, PastConversation,
};
use jig_editor::EditorHandle;

use super::commands::{CommandRun, Preview};
use super::tabs::canonical;
use super::{EditAgents, OpenAgentConversations, OpenCommand, Workspace};

/// Why Agent mode can't start.
const NO_PROJECT: &str =
    "Open the project's folder, or save the file, so the agent knows which project it's in.";

/// What the agent's thread reports to the window.
enum Message {
    Started(Session),
    /// An earlier conversation, picked up again.
    Resumed(Session, Vec<Turn>),
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
enum TurnInput {
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

/// The earlier conversations and OpenCode's commands for one project, kept
/// so the palette can list them at once the next time.
pub(super) struct AgentCatalog {
    /// The agent's key, as Settings > Agent picked it.
    agent: String,
    directory: PathBuf,
    conversations: Vec<jig_ai::agent::Conversation>,
    commands: Vec<AgentCommandInfo>,
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
    /// The agent wants the user to sign in first.
    sign_in: Option<PendingSignIn>,
    /// Kinds of request (their titles, e.g. "Run a command") allowed for
    /// the rest of the conversation, for agents that can't remember that
    /// themselves.
    always_allowed: Vec<String>,
    /// What the conversation is called in OpenCode: what was first asked.
    title: String,
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
    /// The project's folder name, for its header.
    project: String,
    entries: Vec<ChatEntry>,
    /// The reply box, shown between turns.
    pub(super) input: Entity<InputState>,
    scroll: ScrollHandle,
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
    _input_events: Subscription,
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
        let Some(directory) = self.agent_project(cx) else {
            self.chat = None;
            return self.show_note(NO_PROJECT.into(), window, cx);
        };
        // What the user asked, as the conversation's first line.
        let asked = match (&name, &request.comment) {
            (Some(name), Some(note)) => format!("{name}: {note}"),
            (Some(name), None) => name.clone(),
            (None, Some(note)) => format!("{}: {note}", request.instruction.trim()),
            (None, None) => request.instruction.trim().to_string(),
        };
        let agent = self.new_agent_run(name, asked, directory, window, cx);
        let run = self
            .chat
            .as_mut()
            .expect("run_command set up the conversation");
        let target = run.target.clone();
        run.agent = Some(agent);
        let anchor = self.agent_anchor(target, window, cx);
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
            .agent_catalog
            .as_ref()
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
        if let Some(run) = self.chat.as_mut() {
            run.agent = Some(agent);
        }

        let choice = crate::agent::choice(cx);
        let (sender, mut messages) = futures::channel::mpsc::unbounded();
        std::thread::spawn(move || {
            let result = (|| {
                let choice = choice.map_err(anyhow::Error::msg)?;
                let backend = crate::agent::connect(&choice)?;
                let session = backend.open(&directory, &session_id)?;
                let turns = session.history()?;
                anyhow::Ok(Message::Resumed(session, turns))
            })();
            let _ = sender.unbounded_send(result.unwrap_or_else(failure));
        });
        let task = cx.spawn_in(window, async move |this, cx| {
            if let Some(message) = messages.next().await {
                this.update_in(cx, |this, window, cx| {
                    this.on_agent_message(id, message, window, cx)
                })
                .ok();
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
        self.close_chat(window, cx);
        self.next_run_id += 1;
        let id = self.next_run_id;
        let target = self.editor().selection(cx);
        self.chat = Some(CommandRun {
            id,
            bubble: Bubble::Message(String::new()),
            anchor: self.floating_anchor(cx),
            snapshot: String::new(),
            target: target.clone(),
            preview: None,
            agent: None,
            _task: None,
        });
        if self.agent_project(cx).is_none() {
            self.chat = None;
            self.show_note(NO_PROJECT.into(), window, cx);
            return None;
        }
        let anchor = self.agent_anchor(target, window, cx);
        if let Some(run) = self.chat.as_mut() {
            run.anchor = anchor;
        }
        Some(id)
    }

    /// Close the conversation on screen. An edit of its still under review
    /// is undone first: nothing reaches the disk unreviewed. The
    /// conversation stays with the agent, in the history.
    pub(super) fn close_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settle_agent_edit(false, Some(window), cx);
        if self.chat.take().is_some() {
            self.editor().clear_highlights(cx);
            cx.notify();
        }
    }

    /// ⌥⌘K, or the conversation's history button: the palette in Agent
    /// mode, listing the earlier conversations.
    pub(super) fn open_agent_conversations(
        &mut self,
        _: &OpenAgentConversations,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.palette.is_none() {
            self.open_command(&OpenCommand, window, cx);
        }
        let Some(palette) = self.palette.as_ref().map(|palette| palette.view.clone()) else {
            return;
        };
        palette.update(cx, |palette, cx| palette.switch_to_agent(window, cx));
    }

    /// Open `agents.toml`, starting it with a commented example.
    pub(super) fn edit_agents(
        &mut self,
        _: &EditAgents,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = crate::agents::user_path() else {
            return;
        };
        if !path.exists() {
            let created = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, crate::agents::USER_TEMPLATE));
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

    /// Fill the palette's Agent mode: what's known at once, then what
    /// OpenCode says now. Without a palette, just learn the commands, so a
    /// reply can run one.
    pub(super) fn load_agent_catalog(
        &mut self,
        palette: Option<Entity<CommandPalette>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(directory) = self.agent_project(cx) else {
            return;
        };
        let agent = crate::agents::chosen(cx).key.clone();
        if let Some(catalog) = self
            .agent_catalog
            .as_ref()
            .filter(|catalog| catalog.directory == directory && catalog.agent == agent)
            && let Some(palette) = &palette
        {
            show_catalog(catalog, palette, cx);
        }
        let choice = crate::agent::choice(cx);
        let task = cx.background_executor().spawn({
            let directory = directory.clone();
            async move {
                let choice = choice.map_err(anyhow::Error::msg)?;
                let backend = crate::agent::connect(&choice)?;
                let conversations = backend.conversations(&directory)?;
                let commands = backend.commands(&directory)?;
                anyhow::Ok((conversations, commands))
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            // Without OpenCode the palette just has nothing to list; running
            // a prompt says what's wrong.
            let Ok((conversations, commands)) = task.await else {
                return;
            };
            this.update(cx, |this, cx| {
                let catalog = AgentCatalog {
                    agent,
                    directory,
                    conversations,
                    commands: commands
                        .into_iter()
                        .map(|command| AgentCommandInfo {
                            name: command.name,
                            description: command.description,
                            skill: command.skill,
                        })
                        .collect(),
                };
                if let Some(palette) = &palette {
                    show_catalog(&catalog, palette, cx);
                }
                this.agent_catalog = Some(catalog);
            })
            .ok();
        })
        .detach();
    }

    /// A conversation that starts with the user asking `asked`, in the
    /// project around `path`. The agent isn't contacted yet.
    fn new_agent_run(
        &mut self,
        name: Option<String>,
        asked: String,
        directory: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AgentRun {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Reply to the agent…"));
        let input_events =
            cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                if let InputEvent::PressEnter { .. } = event
                    && !this.answer_agent_question(None, window, cx)
                {
                    this.reply_to_agent(window, cx);
                }
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
            sign_in: None,
            always_allowed: Vec::new(),
            title: asked.clone(),
            touched: Vec::new(),
            step: LiveStep::default(),
            label: name.unwrap_or_else(|| crate::agents::chosen(cx).name.clone()),
            started: std::time::Instant::now(),
            context,
            finished: true,
            directory,
            project,
            entries: vec![ChatEntry::User(asked)],
            input,
            scroll: ScrollHandle::new(),
            frame: match self.agent_dock {
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
            _input_events: input_events,
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
        let anchor = self.agent_anchor(self.editor().selection(cx), window, cx);
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
    fn start_turn(
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
        if self.agent_catalog.is_none() {
            self.load_agent_catalog(None, window, cx);
        }
        // Read on this thread: the keys and settings live in the app.
        let choice = crate::agent::choice(cx);
        let Some(agent) = self.agent_run(id) else {
            return;
        };
        let session = agent.session.clone();
        let directory = agent.directory.clone();
        let title = agent.title.clone();
        agent.last_input = Some(input.clone());
        let permissions = crate::settings::get(cx).agent.permissions;
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
                let choice = choice.map_err(anyhow::Error::msg)?;
                let session = match session {
                    Some(session) => session,
                    None => {
                        let backend = crate::agent::connect(&choice)?;
                        let session = backend.create(&directory, &title, &permissions)?;
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
        if let Some(run) = self.chat.as_mut() {
            run.bubble = bubble;
            run._task = Some(task);
        }
        cx.notify();
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
                self.editor().focus(window, cx);
                self.start_turn(id, input, window, cx);
            }
            return;
        }
        agent.sign_in = None;
        agent
            .input
            .update(cx, |input, cx| input.set_value("", window, cx));
        agent.entries.push(ChatEntry::User(text.clone()));
        agent.scroll.scroll_to_bottom();
        // Back to the code, where the agent's edits are reviewed.
        self.editor().focus(window, cx);
        let input = self.slash_command(&text).unwrap_or(TurnInput::Prompt(text));
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
            Some(command) => match crate::agent::open_in_terminal(command) {
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
        let choice = crate::agent::choice(cx);
        let task = cx.background_executor().spawn(async move {
            let backend = crate::agent::connect(&choice.map_err(anyhow::Error::msg)?)?;
            if let jig_ai::harness::Backend::Acp(connection) = backend {
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
    fn slash_command(&self, text: &str) -> Option<TurnInput> {
        let typed = text.strip_prefix('/')?;
        let (name, arguments) = typed.split_once(' ').unwrap_or((typed, ""));
        self.agent_catalog
            .as_ref()?
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
        self.editor().focus(window, cx);
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
        let session = agent.session.clone();
        match agent.pending.take() {
            Some(Pending::Permission(request)) => {
                if !allow {
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
                self.editor().focus(window, cx);
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
            Some(Pending::Permission(request)) => request,
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

    /// One of the request's buttons: 0 allows, 1 allows always, 2 denies.
    fn decide_agent_request(&mut self, button: usize, window: &mut Window, cx: &mut Context<Self>) {
        match button {
            0 => self.settle_agent_request(true, window, cx),
            1 => self.allow_agent_request_always(window, cx),
            _ => self.settle_agent_request(false, window, cx),
        };
        self.editor().focus(window, cx);
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
        let Some(ix) = self
            .agent_edit_elsewhere()
            .and_then(|path| self.tab_for(&path))
        else {
            return false;
        };
        self.activate(ix, window, cx);
        true
    }

    /// The pin button: dock the conversation to the window's right edge, or
    /// float it at the code again.
    fn toggle_agent_chat_pin(&mut self, cx: &mut Context<Self>) {
        if let Some(agent) = self.chat.as_mut().and_then(|run| run.agent.as_mut()) {
            agent.frame.pinned = !agent.frame.pinned;
            self.agent_dock = agent.frame.pinned.then_some(agent.frame.width);
            cx.notify();
        }
    }

    /// Mouse handlers for the whole window, active while the docked
    /// conversation's edge is dragged: it keeps the right edge and takes
    /// what's left of the mouse.
    pub(super) fn agent_dock_drag_handlers(&self, root: Div, cx: &Context<Self>) -> Div {
        const MIN: f32 = 320.;
        root.when(self.resizing_agent_dock, |root| {
            root.cursor_col_resize()
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                    if !event.dragging() {
                        this.resizing_agent_dock = false;
                    } else {
                        let viewport = window.viewport_size().width;
                        // The column's padding on both sides.
                        let width = viewport - event.position.x - px(16.);
                        let width = width.clamp(px(MIN), (viewport * 0.7).max(px(MIN)));
                        if let Some(agent) = this.chat.as_mut().and_then(|run| run.agent.as_mut()) {
                            agent.frame.width = width;
                            this.agent_dock = Some(width);
                        }
                    }
                    cx.notify();
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.resizing_agent_dock = false;
                        cx.notify();
                    }),
                )
        })
    }

    /// The conversation docked to the window's right edge, while pinned.
    pub(super) fn render_pinned_agent_chat(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let agent = self.chat.as_ref()?.agent.as_ref()?;
        if !agent.frame.pinned {
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
                    this.resizing_agent_dock = true;
                    cx.stop_propagation();
                }),
            );
        Some(
            div()
                .relative()
                .flex_none()
                .h_full()
                .p_2()
                .bg(crate::theme::editor_surface(cx))
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

    /// The conversation floating at the code, unless it's pinned.
    pub(super) fn floating_agent_chat_pinned(&self) -> bool {
        self.chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .is_some_and(|agent| agent.frame.pinned)
    }

    /// Esc while the agent works: stop the turn but keep the conversation,
    /// so the user can say what to do instead. Returns false when there is
    /// no turn to stop.
    pub(super) fn stop_agent_turn(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(run) = self.chat.as_mut() else {
            return false;
        };
        let Some(agent) = run.agent.as_mut().filter(|agent| !agent.finished) else {
            return false;
        };
        agent.finished = true;
        agent.queue.clear();
        agent.pending = None;
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
        self.chat
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map(|agent| agent.entries.clone())
            .unwrap_or_default()
    }

    /// Where the conversation opens: just right of the code it's about, so
    /// it doesn't cover it, when the window has room for it there; below
    /// the cursor otherwise.
    fn agent_anchor(&self, range: Range<usize>, window: &Window, cx: &App) -> Point<Pixels> {
        const GAP: f32 = 32.;
        const MARGIN: f32 = 8.;
        let width = self
            .run
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map_or(px(jig_commands::chat::WIDTH), |agent| agent.frame.width);
        self.editor()
            .beside_point(range, cx)
            // Level with the first line, give or take the panel's padding.
            .map(|at| at + point(px(GAP), px(-4.)))
            .filter(|at| at.x + width + px(MARGIN) <= window.viewport_size().width)
            .unwrap_or_else(|| self.floating_anchor(cx))
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

    /// Where the conversation was last drawn, while the run is an agent's.
    pub(super) fn agent_chat_bounds(&self) -> Option<Bounds<Pixels>> {
        Some(self.chat.as_ref()?.agent.as_ref()?.frame.bounds.get())
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
            },
        };
        Some(
            Conversation::new(
                format!("{} · {}", agent.label, agent.project),
                agent.entries.clone(),
                status,
                agent.input.clone(),
                agent.scroll.clone(),
                agent.frame.clone(),
            )
            .on_grab(cx.listener(|this, event, _, cx| this.grab_agent_chat(event, cx)))
            .on_resize_grab(
                cx.listener(|this, event, _, cx| this.grab_agent_chat_corner(event, cx)),
            )
            .on_pin(cx.listener(|this, _, _, cx| this.toggle_agent_chat_pin(cx)))
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
            .into_any_element(),
        )
    }

    fn agent_run(&mut self, id: u64) -> Option<&mut AgentRun> {
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
            Message::Resumed(session, turns) => {
                agent.session = Some(session);
                agent.loading = false;
                agent.entries = turns.into_iter().map(chat_entry).collect();
                agent.scroll.scroll_to_bottom();
                let input = agent.input.clone();
                input.update(cx, |input, cx| input.focus(window, cx));
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
            Message::Event(AgentEvent::Commands(commands)) => {
                let directory = agent.directory.clone();
                self.learn_commands(directory, commands, cx);
            }
            Message::Event(AgentEvent::Did(text)) => {
                agent.entries.push(ChatEntry::Did(text));
                agent.scroll.scroll_to_bottom();
                cx.notify();
            }
            Message::Event(AgentEvent::Done(text)) => {
                agent.finished = true;
                self.finish_agent(text, window, cx);
                return false;
            }
            Message::SignIn(needed) => {
                agent.finished = true;
                agent.loading = false;
                let said = needed.to_string();
                agent.sign_in = Some(PendingSignIn {
                    needed,
                    opened: false,
                });
                self.end_turn(ChatEntry::Note(said), window, cx);
                return false;
            }
            Message::Failed(error) => {
                agent.finished = true;
                agent.loading = false;
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
        cx: &App,
    ) {
        let agent = crate::agents::chosen(cx).key.clone();
        let commands = commands
            .into_iter()
            .map(|command| AgentCommandInfo {
                name: command.name,
                description: command.description,
                skill: command.skill,
            })
            .collect();
        match self
            .agent_catalog
            .as_mut()
            .filter(|catalog| catalog.directory == directory && catalog.agent == agent)
        {
            Some(catalog) => catalog.commands = commands,
            None => {
                self.agent_catalog = Some(AgentCatalog {
                    agent,
                    directory,
                    conversations: Vec::new(),
                    commands,
                })
            }
        }
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
            Some(Request::Edit(edit)) => {
                if let Err(error) = self.show_edit(&edit, window, cx) {
                    // Tell the agent, so it can try again.
                    self.answer_edit(&edit.id, false, Some(&error), cx);
                    self.show_next_request(id, window, cx);
                }
            }
            // Asked in the conversation, answered with the review keys in
            // the code.
            // Allowed always earlier in this conversation.
            Some(Request::Permission(request)) if agent.always_allowed.contains(&request.title) => {
                if let Some(session) = agent.session.clone() {
                    cx.background_executor()
                        .spawn(async move {
                            let _ = session.reply(&request.id, true, None);
                        })
                        .detach();
                }
                self.show_next_request(id, window, cx);
            }
            Some(Request::Permission(request)) => {
                agent.pending = Some(Pending::Permission(request));
                agent.scroll.scroll_to_bottom();
                self.editor().focus(window, cx);
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

    fn show_edit(
        &mut self,
        edit: &EditRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        // A jig's change under review there is kept, as a new command would.
        self.accept_quick_preview(cx);
        let created = !edit.path.exists();
        let disk = std::fs::read_to_string(&edit.path).unwrap_or_default();
        let new = edit.change.apply(&disk).map_err(|error| {
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
            let run = self.chat.take();
            self.open_file(&edit.path, window, cx);
            self.chat = run;
        }
        if self.tab().dirty {
            return Err("The user has unsaved changes in this file; leave it alone.".into());
        }
        self.catch_up_with_disk(self.active, window, cx);

        let old = self.editor().text(cx);
        let preview = crate::diff::preview_edit(&old, &new, 0);
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
            agent: true,
        };
        if let Some(agent) = run.agent.as_mut() {
            agent.pending = Some(Pending::Edit(PendingEdit {
                id: edit.id.clone(),
                path: edit.path.clone(),
                created,
                jig_writes: edit.change.written_by_jig(),
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
        // The bubble follows the change once it's scrolled into view.
        cx.on_next_frame(window, move |this, window, cx| {
            let anchor = this.agent_anchor(range, window, cx);
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
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(run) = self.chat.as_mut() else {
            return false;
        };
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
        } else if let Some(window) = window {
            self.editor().undo(window, cx);
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
        let (range, replacement) = crate::diff::changed_range(&text, &disk);
        tab.editor.apply_edit(range, replacement, window, cx);
        tab.document.saved_text = disk;
        tab.dirty = false;
    }

    fn relative(&self, path: &Path) -> String {
        let root = self
            .run
            .as_ref()
            .and_then(|run| run.agent.as_ref())
            .map(|agent| agent.directory.clone())
            .filter(|root| path.starts_with(root))
            .unwrap_or_else(|| crate::project::root_for(path));
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
                    .unwrap_or_else(|| crate::project::root_for(path)),
            ),
            None => open,
        }
    }
}

/// A line of an earlier conversation, as the transcript shows it.
fn chat_entry(turn: Turn) -> ChatEntry {
    match turn {
        Turn::User(text) => ChatEntry::User(text),
        Turn::Agent(text) => ChatEntry::Agent(text),
        Turn::Edit { path, accepted } => ChatEntry::Edit { path, accepted },
        Turn::Did(text) => ChatEntry::Did(text),
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
