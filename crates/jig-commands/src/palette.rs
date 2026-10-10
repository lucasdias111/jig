//! The floating command input that opens below the cursor.
//!
//! In Jig mode, typing filters the jigs. Enter runs the highlighted row;
//! once something is typed, the first row runs the text itself as a prompt.
//! Jigs set to take a note first switch the input to a note step: Enter
//! runs, Esc goes back to the list. In Agent mode the typed text is a
//! message for the agent, listed above the earlier conversations it
//! matches; "/" lists OpenCode's commands and skills, and "/jig" the jigs.
//! Esc in the list, or clicking away, dismisses it.

use std::rc::Rc;

use gpui_kit::component::input::{
    Backspace, Enter, Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveEnd,
    MoveRight, MoveUp,
};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::presets::{self, CommentMode, Invocation, Preset};

const MAX_ROWS: usize = 8;
const CONTEXT: &str = "JigPalette";

/// What starts the jigs in Agent mode: "/jig simplify".
const JIG_PREFIX: &str = "/jig";

pub enum PaletteEvent {
    Run(Invocation),
    /// Cmd+Enter on typed text: turn it into a preset.
    SaveAsCommand(String),
    /// Pick up the earlier conversation with this id.
    Resume(String),
    /// Run an OpenCode command or skill in a new conversation.
    RunAgentCommand {
        name: String,
        arguments: String,
    },
    /// Switched to Agent mode: the earlier conversations and the commands
    /// are wanted, see [`CommandPalette::set_agent_catalog`].
    AgentMode,
    Dismissed,
}

/// An earlier conversation with the agent, as the palette lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct PastConversation {
    pub id: String,
    pub title: String,
    /// When it last changed, e.g. "2 h ago".
    pub when: String,
}

/// An OpenCode command or skill, run with "/name".
#[derive(Clone, Debug, PartialEq)]
pub struct AgentCommandInfo {
    pub name: String,
    pub description: String,
    pub skill: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Preset(usize),
    Custom(String),
    Conversation(usize),
    Command(usize),
    /// "/jig", listed with the commands.
    Jigs,
}

impl Row {
    fn section(&self) -> &'static str {
        match self {
            Row::Custom(_) => "Prompt",
            Row::Preset(_) => "Jigs",
            Row::Conversation(_) => "Conversations",
            Row::Command(_) | Row::Jigs => "Commands",
        }
    }
}

/// The note step for one preset.
struct NoteStep {
    preset: usize,
    /// The list query to restore when going back.
    query: String,
    /// Enter was pressed on an empty required note.
    missing: bool,
}

pub struct CommandPalette {
    presets: Rc<Vec<Preset>>,
    input: Entity<InputState>,
    rows: Vec<Row>,
    /// The query `rows` were built from.
    filtered_query: String,
    selected: usize,
    has_selection: bool,
    note: Option<NoteStep>,
    /// What a quick command sees besides the instruction, e.g. "AGENTS.md".
    context: Vec<String>,
    /// Tab: hand this run to the coding agent.
    agent: bool,
    /// The text after a colon that follows a command's name, e.g. "terse"
    /// in "docs: terse". `None` without a colon or a matching command.
    inline_note: Option<String>,
    conversations: Vec<PastConversation>,
    commands: Vec<AgentCommandInfo>,
    /// The agent conversation is docked beside the code: Agent mode lives
    /// there meanwhile, so the switch stays on Jig.
    agent_docked: bool,
    _subscription: Subscription,
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

impl Focusable for CommandPalette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl CommandPalette {
    /// `has_selection` decides whether a custom command rewrites the
    /// selection or inserts at the cursor.
    pub fn new(
        presets: Rc<Vec<Preset>>,
        has_selection: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(Self::list_placeholder(has_selection, false))
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change if this.note.is_none() => this.on_query_changed(window, cx),
                InputEvent::Change => {
                    if let Some(note) = this.note.as_mut() {
                        note.missing = false;
                    }
                    cx.notify();
                }
                InputEvent::Blur => cx.emit(PaletteEvent::Dismissed),
                _ => {}
            },
        );
        input.update(cx, |input, cx| input.focus(window, cx));
        let mut this = Self {
            presets,
            input,
            rows: Vec::new(),
            filtered_query: String::new(),
            selected: 0,
            has_selection,
            note: None,
            context: Vec::new(),
            agent: false,
            inline_note: None,
            conversations: Vec::new(),
            commands: Vec::new(),
            agent_docked: false,
            _subscription: subscription,
        };
        this.refilter(cx);
        this
    }

    fn list_placeholder(has_selection: bool, agent: bool) -> &'static str {
        match (agent, has_selection) {
            (false, true) => "Jig or prompt for the selection…",
            (false, false) => "Jig or prompt…",
            (true, true) => "Ask the agent about the selection, or / for commands…",
            (true, false) => "Ask the agent, or / for commands…",
        }
    }

    /// Kept to Jig mode, the agent being docked.
    pub fn is_jig_only(&self) -> bool {
        self.agent_docked && !self.agent
    }

    /// Keep to Jig mode while the agent conversation is docked.
    pub fn with_agent_docked(mut self, docked: bool) -> Self {
        self.agent_docked = docked;
        self
    }

    /// Switch to Agent mode, e.g. to pick up an earlier conversation. This
    /// works while the conversation is docked too: it's how to pick another.
    pub fn switch_to_agent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.agent_docked = false;
        self.set_agent(true, window, cx);
    }

    /// The earlier conversations and OpenCode's commands, listed in Agent
    /// mode once they've been fetched.
    pub fn set_agent_catalog(
        &mut self,
        conversations: Vec<PastConversation>,
        commands: Vec<AgentCommandInfo>,
        cx: &mut Context<Self>,
    ) {
        self.conversations = conversations;
        self.commands = commands;
        if self.note.is_none() {
            let selected = self.rows.get(self.selected).cloned();
            self.refilter(cx);
            // Keep the arrows' choice when the list fills in under it.
            if let Some(index) = selected.and_then(|row| self.rows.iter().position(|r| *r == row)) {
                self.selected = index;
            }
        }
    }

    /// The part of `query` that names a jig: all of it in Jig mode, the
    /// rest after "/jig " in Agent mode, `None` when it's for the agent.
    fn jig_query<'a>(&self, query: &'a str) -> Option<&'a str> {
        if !self.agent {
            return Some(query);
        }
        let rest = query.trim_start().strip_prefix(JIG_PREFIX)?;
        if rest.is_empty() {
            Some(rest)
        } else {
            rest.strip_prefix(' ')
        }
    }

    /// The highlighted command is set to run on the agent, whatever the
    /// switch says.
    fn agent_by_command(&self) -> bool {
        self.note
            .as_ref()
            .map(|note| note.preset)
            .or_else(|| match self.rows.get(self.selected) {
                Some(Row::Preset(preset)) => Some(*preset),
                _ => None,
            })
            .is_some_and(|preset| self.presets[preset].agent)
    }

    /// Whether Enter would hand the run to the agent.
    fn agent_lane(&self) -> bool {
        self.agent || self.agent_by_command()
    }

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// The project files a quick command is sent along with the file, named
    /// in the footer.
    pub fn with_context(mut self, context: Vec<String>) -> Self {
        self.context = context;
        self
    }

    fn on_query_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query(cx);
        // ":" right after choosing a command with the arrows completes its
        // name, so the note that follows goes to that command.
        if let Some(before) = query.strip_suffix(':')
            && before == self.filtered_query
            && let Some(jig) = self.jig_query(before)
            && !jig.contains(':')
            && let Some(Row::Preset(preset)) = self.rows.get(self.selected)
        {
            let prefix = &before[..before.len() - jig.len()];
            let completed = format!("{prefix}{}: ", self.presets[*preset].name);
            self.input
                .update(cx, |input, cx| input.set_value(completed, window, cx));
        }
        self.refilter(cx);
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        match self.jig_query(&query) {
            Some(jig) => {
                let jig = jig.to_string();
                self.filter_jigs(&jig, !self.agent);
            }
            None if query.trim_start().starts_with('/') => self.filter_commands(&query),
            None => self.filter_conversations(&query),
        }
        self.filtered_query = query;
        cx.notify();
    }

    /// Agent mode: the typed message, then the earlier conversations it
    /// matches (all of them before anything is typed).
    fn filter_conversations(&mut self, query: &str) {
        self.inline_note = None;
        let query = query.trim();
        let lower = query.to_lowercase();
        self.rows = Vec::new();
        if !query.is_empty() {
            self.rows.push(Row::Custom(query.to_string()));
        }
        self.rows.extend(
            self.conversations
                .iter()
                .enumerate()
                .filter(|(_, conversation)| {
                    lower
                        .split_whitespace()
                        .all(|word| conversation.title.to_lowercase().contains(word))
                })
                .map(|(index, _)| Row::Conversation(index)),
        );
        self.selected = 0;
    }

    /// Agent mode after "/": "/jig", then the commands and skills whose
    /// names match. Text after the name is the command's arguments. With
    /// nothing matching, the text goes to the agent as it is.
    fn filter_commands(&mut self, query: &str) {
        self.inline_note = None;
        let typed = query.trim_start().trim_start_matches('/');
        let (name, arguments) = typed.split_once(' ').unwrap_or((typed, ""));
        let name = name.to_lowercase();
        self.rows = Vec::new();
        if arguments.is_empty() && JIG_PREFIX[1..].starts_with(&name) {
            self.rows.push(Row::Jigs);
        }
        let mut matches: Vec<(u32, usize)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let own = command.name.to_lowercase();
                if arguments.is_empty() {
                    crate::presets::score(&own, &name).map(|score| (score, index))
                } else {
                    (own == name).then_some((0, index))
                }
            })
            .collect();
        matches.sort();
        self.rows
            .extend(matches.into_iter().map(|(_, index)| Row::Command(index)));
        if self.rows.is_empty() {
            self.rows.push(Row::Custom(query.trim().to_string()));
        }
        self.selected = 0;
    }

    /// The jigs `query` matches, after the typed text as a prompt when
    /// `with_prompt` is set.
    fn filter_jigs(&mut self, query: &str, with_prompt: bool) {
        // "docs: terse" is the Add docs command with the note "terse", as
        // long as something matches "docs".
        let inline = query.split_once(':').and_then(|(name, note)| {
            let matches = presets::filter(&self.presets, name);
            (!name.trim().is_empty() && !matches.is_empty())
                .then(|| (matches, note.trim().to_string()))
        });
        let (matches, named) = match inline {
            Some((matches, note)) => {
                self.inline_note = Some(note);
                (matches, true)
            }
            None => {
                self.inline_note = None;
                let matches = presets::filter(&self.presets, query);
                let named = matches
                    .first()
                    .is_some_and(|&first| presets::names(&self.presets[first], query));
                (matches, named)
            }
        };
        // The typed text as a prompt first, then the commands it matches.
        // Enter takes the command only when the text clearly names it.
        self.rows = Vec::new();
        if with_prompt && !query.trim().is_empty() {
            self.rows.push(Row::Custom(query.trim().to_string()));
        }
        let has_prompt = !self.rows.is_empty();
        self.rows.extend(matches.into_iter().map(Row::Preset));
        self.selected = if named && has_prompt { 1 } else { 0 };
    }

    /// The rest of the highlighted jig's name when the text typed so far is
    /// how it starts and the cursor is at its end: "ain" after "Expl".
    fn completion(&self, cx: &App) -> Option<String> {
        let input = self.input.read(cx);
        let query = input.value();
        let at_end = input.selected_range() == (query.len()..query.len());
        if self.note.is_some() || !at_end {
            return None;
        }
        let command = |name: &str| -> Option<String> {
            let typed = query.trim_start().strip_prefix('/')?;
            if typed.contains(' ') {
                return None;
            }
            presets::completion(name, typed).map(str::to_string)
        };
        match self.rows.get(self.selected)? {
            Row::Preset(preset) => {
                let jig = self.jig_query(&query)?;
                if jig.contains(':') {
                    return None;
                }
                presets::completion(&self.presets[*preset].name, jig).map(str::to_string)
            }
            Row::Command(index) => command(&self.commands[*index].name),
            Row::Jigs => command(&JIG_PREFIX[1..]),
            _ => None,
        }
    }

    /// → or End at the end of the text completes the highlighted jig's name;
    /// anywhere else they move the cursor as usual.
    fn complete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_filtered(cx);
        let Some(rest) = self.completion(cx) else {
            return;
        };
        cx.stop_propagation();
        self.input
            .update(cx, |input, cx| input.insert(rest, window, cx));
        self.refilter(cx);
    }

    fn on_right(&mut self, _: &MoveRight, window: &mut Window, cx: &mut Context<Self>) {
        self.complete(window, cx);
    }

    fn on_end(&mut self, _: &MoveEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.complete(window, cx);
    }

    /// Text typed in the same frame as Enter or Tab hasn't been filtered
    /// yet; do it now so the right row is chosen.
    fn ensure_filtered(&mut self, cx: &mut Context<Self>) {
        if self.query(cx) != self.filtered_query {
            self.refilter(cx);
        }
    }

    /// Run the row at `index`, or open its note step if the command asks
    /// for one.
    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let note = self.inline_note.clone().unwrap_or_default();
        match self.rows.get(index) {
            Some(Row::Preset(preset))
                if self.presets[*preset].comment == CommentMode::Required && note.is_empty()
                    || self.presets[*preset].comment == CommentMode::Optional
                        && self.inline_note.is_none() =>
            {
                self.start_note(*preset, window, cx)
            }
            Some(Row::Preset(preset)) => cx.emit(PaletteEvent::Run(
                Invocation::preset(&self.presets[*preset])
                    .with_comment(&note)
                    .on_agent(self.agent),
            )),
            Some(Row::Custom(text)) => cx.emit(PaletteEvent::Run(
                Invocation::custom(text, self.has_selection).on_agent(self.agent),
            )),
            Some(Row::Conversation(index)) => {
                cx.emit(PaletteEvent::Resume(self.conversations[*index].id.clone()))
            }
            Some(Row::Command(index)) => {
                let query = self.query(cx);
                let arguments = query
                    .trim_start()
                    .split_once(' ')
                    .map(|(_, arguments)| arguments.trim().to_string())
                    .unwrap_or_default();
                cx.emit(PaletteEvent::RunAgentCommand {
                    name: self.commands[*index].name.clone(),
                    arguments,
                })
            }
            // Choosing "/jig" lists the jigs.
            Some(Row::Jigs) => {
                let text = format!("{JIG_PREFIX} ");
                self.input
                    .update(cx, |input, cx| input.set_value(text, window, cx));
                self.refilter(cx);
            }
            None => {}
        }
    }

    fn start_note(&mut self, preset: usize, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let command = &self.presets[preset];
        let placeholder = match (&command.comment_hint, command.comment) {
            (Some(hint), _) => format!("{hint}…"),
            (None, CommentMode::Required) => "Add a note…".to_string(),
            (None, _) => "Add a note (optional)…".to_string(),
        };
        self.note = Some(NoteStep {
            preset,
            query,
            missing: false,
        });
        self.input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.set_placeholder(placeholder, window, cx);
        });
        cx.notify();
    }

    fn back_to_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(note) = self.note.take() else { return };
        let placeholder = Self::list_placeholder(self.has_selection, self.agent);
        self.input.update(cx, |input, cx| {
            input.set_value(note.query, window, cx);
            input.set_placeholder(placeholder, window, cx);
        });
        self.refilter(cx);
        if let Some(index) = self
            .rows
            .iter()
            .position(|row| *row == Row::Preset(note.preset))
        {
            self.selected = index;
        }
        cx.notify();
    }

    fn run_note(&mut self, cx: &mut Context<Self>) {
        let Some(note) = self.note.as_mut() else {
            return;
        };
        let preset = &self.presets[note.preset];
        let text = self.input.read(cx).value().to_string();
        if preset.comment == CommentMode::Required && text.trim().is_empty() {
            note.missing = true;
            cx.notify();
            return;
        }
        cx.emit(PaletteEvent::Run(
            Invocation::preset(preset)
                .with_comment(&text)
                .on_agent(self.agent),
        ));
    }

    fn on_enter(&mut self, action: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.note.is_some() {
            self.run_note(cx);
        } else if action.secondary {
            let query = self.query(cx);
            if !query.trim().is_empty() {
                cx.emit(PaletteEvent::SaveAsCommand(query.trim().to_string()));
            }
        } else {
            self.ensure_filtered(cx);
            self.choose(self.selected, window, cx);
        }
    }

    /// Tab switches between Jig and Agent mode, unless the agent is docked.
    fn on_tab(&mut self, _: &IndentInline, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if !self.agent_docked {
            self.set_agent(!self.agent, window, cx);
        }
    }

    /// Backspace in an empty note goes back to the list.
    fn on_backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.note.is_some() && self.query(cx).is_empty() {
            cx.stop_propagation();
            self.back_to_list(window, cx);
        }
    }

    fn set_agent(&mut self, agent: bool, window: &mut Window, cx: &mut Context<Self>) {
        let switched = self.agent != agent;
        self.agent = agent;
        if self.note.is_none() {
            let placeholder = Self::list_placeholder(self.has_selection, agent);
            self.input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
            self.refilter(cx);
        }
        if switched && agent {
            cx.emit(PaletteEvent::AgentMode);
        }
        cx.notify();
    }

    /// Jig | Agent, as a segmented control.
    fn render_lane_switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let agent = self.agent_lane();
        let segment = |lane: bool, cx: &mut Context<Self>| {
            let theme = cx.theme();
            let accent = crate::surface::lane_accent(lane, cx);
            let active = lane == agent;
            let locked = lane && self.agent_docked;
            h_flex()
                .id(if lane {
                    "jig-lane-agent"
                } else {
                    "jig-lane-jig"
                })
                .gap_1()
                .px_2()
                .h(px(22.))
                .rounded(px(5.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .child(
                    crate::surface::lane_icon(lane)
                        .size(px(12.))
                        .text_color(if active {
                            accent
                        } else {
                            theme.muted_foreground
                        }),
                )
                .child(crate::surface::lane_name(lane))
                .when(active, |this| {
                    this.bg(accent.opacity(0.16)).text_color(accent)
                })
                .when(!active && !locked, |this| {
                    this.text_color(theme.muted_foreground)
                        .hover(|this| this.bg(theme.foreground.opacity(0.06)))
                })
                .when(locked, |this| {
                    this.text_color(theme.muted_foreground).opacity(0.45)
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if !locked {
                            this.set_agent(lane, window, cx);
                        }
                    }),
                )
        };
        let right = if self.agent_by_command() && !self.agent {
            crate::surface::hint("set by this jig", cx)
        } else if self.agent_docked {
            crate::surface::hint("the agent is in its panel", cx)
        } else {
            crate::surface::hint("⇥ switch", cx)
        };
        h_flex()
            .px_1()
            .pb_0p5()
            .justify_between()
            .child(
                h_flex()
                    .p(px(2.))
                    .gap(px(2.))
                    .rounded(px(7.))
                    .bg(theme.foreground.opacity(0.05))
                    .child(segment(false, cx))
                    .child(segment(true, cx)),
            )
            .child(right)
    }

    fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.note.is_some() {
            self.back_to_list(window, cx);
        } else {
            cx.emit(PaletteEvent::Dismissed);
        }
    }

    fn on_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.note.is_none() && !self.rows.is_empty() {
            self.selected = (self.selected + self.rows.len() - 1) % self.rows.len();
            cx.notify();
        }
    }

    fn on_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.note.is_none() && !self.rows.is_empty() {
            self.selected = (self.selected + 1) % self.rows.len();
            cx.notify();
        }
    }

    /// Spotlight-sized input, marked with the lane it runs on, with the rest
    /// of a jig's name suggested after the typed text.
    fn render_input(&self, agent: bool, accent: Hsla, cx: &mut Context<Self>) -> impl IntoElement {
        let completion = self.completion(cx);
        let muted = cx.theme().muted_foreground;
        div()
            .relative()
            .px_1()
            .text_size(px(15.))
            .child(
                Input::new(&self.input)
                    .appearance(false)
                    .cleanable(false)
                    .prefix(
                        crate::surface::lane_icon(agent)
                            .size(px(15.))
                            .text_color(accent),
                    )
                    .when(completion.is_some(), |this| {
                        this.suffix(crate::surface::hint("→ complete", cx))
                    }),
            )
            .children(completion.map(|rest| suggestion(self.input.clone(), rest, muted)))
    }

    fn render_row(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = index == self.selected;
        let (label, detail): (SharedString, SharedString) = match row {
            Row::Preset(i) => {
                let preset = &self.presets[*i];
                let detail = match preset.comment {
                    _ if self.inline_note.is_some() => preset.scope.label().to_string(),
                    CommentMode::None if selected => {
                        format!("{} · “:” adds a note", preset.scope.label())
                    }
                    CommentMode::None => preset.scope.label().to_string(),
                    _ => format!("{} · note", preset.scope.label()),
                };
                (preset.name.clone().into(), detail.into())
            }
            Row::Custom(text) => (
                format!("“{text}”").into(),
                if selected && !self.agent {
                    "⌘↩ save as jig".into()
                } else {
                    "".into()
                },
            ),
            Row::Conversation(index) => {
                let conversation = &self.conversations[*index];
                (
                    conversation.title.clone().into(),
                    conversation.when.clone().into(),
                )
            }
            Row::Command(index) => {
                let command = &self.commands[*index];
                let kind = if command.skill { "skill" } else { "command" };
                (format!("/{}", command.name).into(), kind.into())
            }
            Row::Jigs => (JIG_PREFIX.into(), "run a jig on the agent".into()),
        };
        // What a command does, after its name.
        let description = match row {
            Row::Command(index) => Some(self.commands[*index].description.clone()),
            _ => None,
        }
        .filter(|description| !description.is_empty())
        .map(|description| {
            div()
                .min_w_0()
                .truncate()
                .text_size(px(12.))
                .opacity(0.7)
                .child(description)
        });
        let row_agent = self.agent || matches!(row, Row::Preset(i) if self.presets[*i].agent);
        let accent = crate::surface::lane_accent(row_agent, cx);
        let theme = cx.theme();
        // Commands that always use the agent say so wherever they're listed.
        let tag = matches!(row, Row::Preset(i) if self.presets[*i].agent)
            .then(|| crate::surface::lane_tag(true, cx));
        // The note typed after the colon, shown with the command it goes to.
        let note = self
            .inline_note
            .clone()
            .filter(|note| !note.is_empty() && matches!(row, Row::Preset(_)))
            .map(|note| {
                div()
                    .px_1p5()
                    .rounded(px(4.))
                    .truncate()
                    .text_size(px(12.))
                    .bg(theme.foreground.opacity(0.07))
                    .when(selected, |this| {
                        this.bg(theme.primary_foreground.opacity(0.2))
                    })
                    .child(note)
            });
        // Selected like a macOS menu item: accent fill, white text.
        h_flex()
            .id(("jig-command", index))
            .h(px(28.))
            .px_2p5()
            .gap_2()
            .rounded(px(6.))
            .justify_between()
            .text_size(px(13.))
            .text_color(theme.popover_foreground)
            .when(selected, |row| {
                row.bg(accent).text_color(theme.primary_foreground)
            })
            .when(!selected, |row| {
                row.hover(|row| row.bg(theme.foreground.opacity(0.06)))
            })
            .child(
                h_flex()
                    .gap_1p5()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .when(note.is_some() || description.is_some(), |this| {
                                this.flex_none()
                            })
                            .child(label),
                    )
                    .children(note)
                    .children(description)
                    .children(tag.map(|tag| {
                        tag.when(selected, |tag| {
                            tag.bg(theme.primary_foreground.opacity(0.2))
                                .text_color(theme.primary_foreground)
                        })
                    })),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .when(selected, |this| {
                        this.text_color(theme.primary_foreground.opacity(0.8))
                    })
                    .when(!selected, |this| this.text_color(theme.muted_foreground))
                    .child(detail),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.choose(index, window, cx);
                }),
            )
    }

    /// The command being annotated, shown above the note input.
    fn render_note_header(&self, note: &NoteStep, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let preset = &self.presets[note.preset];
        let requirement = match preset.comment {
            CommentMode::Required => "note required",
            _ => "note optional",
        };
        h_flex()
            .px_2p5()
            .pt_1p5()
            .gap_2()
            .text_size(px(11.))
            .child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded(px(5.))
                    .bg(theme.primary.opacity(0.15))
                    .text_color(theme.popover_foreground)
                    .font_weight(FontWeight::MEDIUM)
                    .child(preset.name.clone()),
            )
            .child(
                div()
                    .text_color(if note.missing {
                        theme.danger
                    } else {
                        theme.muted_foreground
                    })
                    .child(format!("{} · {requirement}", preset.scope.label())),
            )
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let first = self.selected.saturating_sub(MAX_ROWS - 1);
        let rows: Vec<_> = if self.note.is_some() {
            Vec::new()
        } else {
            self.rows
                .clone()
                .iter()
                .enumerate()
                .skip(first)
                .take(MAX_ROWS)
                .flat_map(|(index, row)| {
                    // Label each kind of row where it starts: the typed
                    // prompt, and the jigs, commands or conversations below.
                    let section = (index == first
                        || self.rows[index - 1].section() != row.section())
                    .then(|| row.section());
                    let header = section.map(|title| {
                        div()
                            .px_2p5()
                            .pt_1()
                            .pb_0p5()
                            .text_size(px(10.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(cx.theme().muted_foreground)
                            .child(title.to_uppercase())
                            .into_any_element()
                    });
                    header
                        .into_iter()
                        .chain([self.render_row(index, row, cx).into_any_element()])
                })
                .collect()
        };
        let header = self
            .note
            .as_ref()
            .map(|note| self.render_note_header(note, cx).into_any_element());
        let switch = self.render_lane_switch(cx).into_any_element();
        let agent = self.agent_lane();
        let accent = crate::surface::lane_accent(agent, cx);
        let input = self.render_input(agent, accent, cx).into_any_element();
        let theme = cx.theme();
        let footer = if agent {
            match self.rows.get(self.selected) {
                Some(Row::Conversation(_)) => "Picks up this conversation where it left off".into(),
                Some(Row::Command(_)) => {
                    "Runs in a new conversation · you review every edit".to_string()
                }
                _ => "Works across the project · you review every edit".to_string(),
            }
        } else {
            let target = if self.has_selection {
                "the selection"
            } else {
                "the cursor"
            };
            let sees = std::iter::once("this file")
                .chain(self.context.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(", ");
            format!("Edits {target} · sees {sees}")
        };

        let palette = crate::surface::panel(cx)
            .flex()
            .flex_col()
            .key_context(CONTEXT)
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_up))
            .capture_action(cx.listener(Self::on_down))
            .capture_action(cx.listener(Self::on_tab))
            .capture_action(cx.listener(Self::on_right))
            .capture_action(cx.listener(Self::on_end))
            .capture_action(cx.listener(Self::on_backspace))
            .w(px(420.))
            .p_1p5()
            .gap_0p5()
            .when(agent, |this| this.border_color(accent.opacity(0.55)))
            .child(switch)
            .children(header)
            .child(input)
            .when(self.note.is_some(), |this| {
                this.child(
                    div()
                        .px_2p5()
                        .pb_1()
                        .child(crate::surface::hint("↩ run · esc back", cx)),
                )
            })
            .when(!rows.is_empty(), |this| {
                this.child(
                    div()
                        .h(px(1.))
                        .mx_neg_1p5()
                        .my_1()
                        .bg(theme.foreground.opacity(0.08)),
                )
                .children(rows)
            })
            .child(
                div()
                    .px_2p5()
                    .pt_1()
                    .pb_0p5()
                    .text_size(px(11.))
                    .text_color(if agent {
                        accent
                    } else {
                        theme.muted_foreground
                    })
                    .child(footer),
            );
        crate::motion::pop_in(palette, "jig-palette")
    }
}

/// The rest of a jig's name in grey after the typed text. Painted after the
/// input, so it goes where the input has just laid its text out, in the same
/// font at the size the input actually drew.
fn suggestion(input: Entity<InputState>, rest: String, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, cx| {
            let state = input.read(cx);
            let typed = state.value();
            let (Some(drawn), Some(end)) = (
                state.range_to_bounds(&(0..typed.len())),
                state.range_to_bounds(&(typed.len()..typed.len())),
            ) else {
                return;
            };
            let style = window.text_style();
            let size = style.font_size.to_pixels(window.rem_size());
            let measured = window.text_system().shape_line(
                typed.clone(),
                size,
                &[style.to_run(typed.len())],
                None,
            );
            let size = if measured.width > px(0.) {
                size * (drawn.size.width / measured.width)
            } else {
                size
            };
            let run = TextRun {
                color,
                ..style.to_run(rest.len())
            };
            let line = window
                .text_system()
                .shape_line(rest.into(), size, &[run], None);
            let _ = line.paint(
                end.origin,
                end.size.height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        },
    )
    .absolute()
    .size_full()
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Point, Render, Styled,
        Subscription, TestAppContext, Window, WindowBounds, WindowOptions, div, px, size,
    };

    use super::{CommandPalette, PaletteEvent};
    use crate::presets::{self, Invocation, Scope};

    struct Host {
        palette: Entity<CommandPalette>,
        events: Vec<Option<Invocation>>,
        /// Everything else it said, by kind: "resume ses_1", "command review main".
        other: Vec<String>,
        _subscription: Subscription,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.palette.clone())
        }
    }

    fn open(
        cx: &mut TestAppContext,
        has_selection: bool,
    ) -> (gpui_kit::AnyWindowHandle, Entity<Host>) {
        open_with(cx, has_selection, presets::defaults())
    }

    fn open_with(
        cx: &mut TestAppContext,
        has_selection: bool,
        presets: Vec<presets::Preset>,
    ) -> (gpui_kit::AnyWindowHandle, Entity<Host>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
        });
        let handles = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(600.), px(400.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let presets = Rc::new(presets);
                    let palette =
                        cx.new(|cx| CommandPalette::new(presets, has_selection, window, cx));
                    let subscription =
                        cx.subscribe(&palette, |host: &mut Host, _, event: &PaletteEvent, _| {
                            match event {
                                PaletteEvent::Run(invocation) => {
                                    host.events.push(Some(invocation.clone()))
                                }
                                PaletteEvent::Dismissed | PaletteEvent::SaveAsCommand(_) => {
                                    host.events.push(None)
                                }
                                PaletteEvent::Resume(id) => host.other.push(format!("resume {id}")),
                                PaletteEvent::RunAgentCommand { name, arguments } => host
                                    .other
                                    .push(format!("command {name} {arguments}").trim().into()),
                                PaletteEvent::AgentMode => host.other.push("agent mode".into()),
                            }
                        });
                    Host {
                        palette,
                        events: Vec::new(),
                        other: Vec::new(),
                        _subscription: subscription,
                    }
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        handles
    }

    fn step(
        cx: &mut TestAppContext,
        window: gpui_kit::AnyWindowHandle,
        f: impl FnOnce(&mut Window, &mut gpui_kit::App),
    ) {
        cx.update_window(window, |_, window, cx| f(window, cx))
            .unwrap();
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn enter_runs_best_match(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("docs", cx);
        });
        step(cx, window, |window, cx| window.press("enter", cx));
        step(cx, window, |_, cx| {
            let events = &host.read(cx).events;
            assert_eq!(events.len(), 1);
            let invocation = events[0].clone().unwrap();
            assert_eq!(invocation.name.as_deref(), Some("Add docs"));
        });
    }

    #[gpui_kit::test]
    fn arrows_reach_custom_row(cx: &mut TestAppContext) {
        let (window, host) = open(cx, false);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("add", cx);
        });
        // "Add docs", "Add tests", then the custom row; up wraps to the last.
        step(cx, window, |window, cx| {
            window.press("up", cx);
            window.press("enter", cx);
        });
        step(cx, window, |_, cx| {
            let invocation = host.read(cx).events[0].clone().unwrap();
            assert_eq!(
                invocation,
                Invocation {
                    comment: None,
                    agent: false,
                    diagnostics: false,
                    name: None,
                    instruction: "add".into(),
                    scope: Scope::Cursor
                }
            );
        });
    }

    #[gpui_kit::test]
    fn unmatched_text_runs_as_custom(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("make this async", cx);
        });
        step(cx, window, |window, cx| window.press("enter", cx));
        step(cx, window, |_, cx| {
            let invocation = host.read(cx).events[0].clone().unwrap();
            assert_eq!(invocation.name, None);
            assert_eq!(invocation.scope, Scope::Selection);
        });
    }

    #[gpui_kit::test]
    fn escape_dismisses(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("escape", cx);
        });
        step(cx, window, |_, cx| {
            assert_eq!(host.read(cx).events.first(), Some(&None))
        });
    }

    fn note_presets() -> Vec<presets::Preset> {
        presets::parse(
            r#"
            [[jig]]
            name = "Create controller"
            scope = "cursor"
            prompt = "Insert a controller."
            comment = "required"
            comment_hint = "Entity name"

            [[jig]]
            name = "Rename"
            prompt = "Rename this."
            comment = "optional"

            [[jig]]
            name = "Simplify"
            prompt = "Simplify this."
            "#,
        )
        .unwrap()
    }

    fn events(cx: &mut TestAppContext, host: &Entity<Host>) -> Vec<Option<Invocation>> {
        cx.update(|cx| host.read(cx).events.clone())
    }

    fn in_note_step(cx: &mut TestAppContext, host: &Entity<Host>) -> bool {
        cx.update(|cx| host.read(cx).palette.read(cx).note.is_some())
    }

    #[gpui_kit::test]
    fn required_note_must_be_given(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, false, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("controller", cx);
            window.press("enter", cx);
        });
        assert!(in_note_step(cx, &host), "Enter asks for the note first");
        assert!(events(cx, &host).is_empty());

        step(cx, window, |window, cx| window.press("enter", cx));
        assert!(
            events(cx, &host).is_empty(),
            "an empty required note doesn't run"
        );

        step(cx, window, |window, cx| {
            window.input("User", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name.as_deref(), Some("Create controller"));
        assert_eq!(invocation.comment.as_deref(), Some("User"));
        assert_eq!(
            invocation.instruction, "Insert a controller.",
            "the base prompt is unchanged"
        );
    }

    #[gpui_kit::test]
    fn optional_note_can_be_skipped(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, true, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("rename", cx);
            window.press("enter", cx);
        });
        assert!(in_note_step(cx, &host));
        step(cx, window, |window, cx| window.press("enter", cx));
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name.as_deref(), Some("Rename"));
        assert_eq!(invocation.comment, None);
    }

    #[gpui_kit::test]
    fn a_loose_match_runs_the_prompt(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, true, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            // A subsequence of "Simplify", but not its name: a prompt.
            window.input("smp", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name, None);
        assert_eq!(invocation.instruction, "smp");
    }

    #[gpui_kit::test]
    fn the_command_is_one_arrow_below_the_prompt(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, true, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("smp", cx);
        });
        step(cx, window, |window, cx| {
            window.press("down", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name.as_deref(), Some("Simplify"));
    }

    #[gpui_kit::test]
    fn a_colon_completes_the_command_chosen_with_the_arrows(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, true, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("smp", cx);
        });
        // The prompt is highlighted; ↓ picks Simplify.
        step(cx, window, |window, cx| window.press("down", cx));
        step(cx, window, |window, cx| window.input(":", cx));
        cx.update(|cx| {
            assert_eq!(host.read(cx).palette.read(cx).query(cx), "Simplify: ");
        });
        step(cx, window, |window, cx| {
            window.input("keep the early return", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name.as_deref(), Some("Simplify"));
        assert_eq!(invocation.comment.as_deref(), Some("keep the early return"));
    }

    fn query(cx: &mut TestAppContext, host: &Entity<Host>) -> String {
        cx.update(|cx| host.read(cx).palette.read(cx).query(cx))
    }

    #[gpui_kit::test]
    fn right_completes_the_named_jig(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| window.input("expl", cx));
        step(cx, window, |window, cx| window.press("right", cx));
        assert_eq!(query(cx, &host), "explain");
        // Typing goes on after the completed name, and it still runs.
        step(cx, window, |window, cx| window.input(":", cx));
        assert_eq!(query(cx, &host), "Explain: ");
    }

    #[gpui_kit::test]
    fn end_completes_a_jig_after_slash_jig_in_agent_mode(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| window.press("tab", cx));
        step(cx, window, |window, cx| window.input("/jig Add t", cx));
        step(cx, window, |window, cx| window.press("end", cx));
        assert_eq!(query(cx, &host), "/jig Add tests");
    }

    /// Agent mode with two earlier conversations and two commands listed.
    fn open_agent(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Entity<Host>) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| window.press("tab", cx));
        cx.update(|cx| {
            let palette = host.read(cx).palette.clone();
            palette.update(cx, |palette, cx| {
                palette.set_agent_catalog(
                    vec![
                        super::PastConversation {
                            id: "ses_1".into(),
                            title: "Add docs to the parser".into(),
                            when: "2 h ago".into(),
                        },
                        super::PastConversation {
                            id: "ses_2".into(),
                            title: "Fix the flaky test".into(),
                            when: "yesterday".into(),
                        },
                    ],
                    vec![
                        super::AgentCommandInfo {
                            name: "review".into(),
                            description: "review changes".into(),
                            skill: false,
                        },
                        super::AgentCommandInfo {
                            name: "init".into(),
                            description: "guided AGENTS.md setup".into(),
                            skill: false,
                        },
                    ],
                    cx,
                )
            })
        });
        cx.run_until_parked();
        (window, host)
    }

    fn rows(cx: &mut TestAppContext, host: &Entity<Host>) -> Vec<super::Row> {
        cx.update(|cx| host.read(cx).palette.read(cx).rows.clone())
    }

    #[gpui_kit::test]
    fn agent_mode_lists_earlier_conversations_instead_of_jigs(cx: &mut TestAppContext) {
        let (window, host) = open_agent(cx);
        assert_eq!(
            cx.update(|cx| host.read(cx).other.clone()),
            ["agent mode"],
            "switching asks for the conversations"
        );
        assert_eq!(
            rows(cx, &host),
            [super::Row::Conversation(0), super::Row::Conversation(1)]
        );
        // Text is a message for the agent, above the conversations it matches.
        step(cx, window, |window, cx| window.input("flaky", cx));
        assert_eq!(
            rows(cx, &host),
            [
                super::Row::Custom("flaky".into()),
                super::Row::Conversation(1)
            ]
        );
        step(cx, window, |window, cx| {
            window.press("down", cx);
            window.press("enter", cx);
        });
        assert_eq!(
            cx.update(|cx| host.read(cx).other.clone()),
            ["agent mode", "resume ses_2"]
        );
    }

    #[gpui_kit::test]
    fn typing_in_agent_mode_runs_a_prompt_even_when_it_names_a_jig(cx: &mut TestAppContext) {
        let (window, host) = open_agent(cx);
        step(cx, window, |window, cx| window.input("explain", cx));
        assert_eq!(rows(cx, &host), [super::Row::Custom("explain".into())]);
        step(cx, window, |window, cx| window.press("enter", cx));
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name, None);
        assert!(invocation.agent);
    }

    #[gpui_kit::test]
    fn slash_lists_commands_and_runs_one_with_arguments(cx: &mut TestAppContext) {
        let (window, host) = open_agent(cx);
        step(cx, window, |window, cx| window.input("/", cx));
        assert_eq!(
            rows(cx, &host),
            [
                super::Row::Jigs,
                super::Row::Command(0),
                super::Row::Command(1)
            ]
        );
        step(cx, window, |window, cx| window.input("rev", cx));
        assert_eq!(rows(cx, &host), [super::Row::Command(0)]);
        step(cx, window, |window, cx| window.press("right", cx));
        assert_eq!(query(cx, &host), "/review");
        step(cx, window, |window, cx| window.input(" main", cx));
        step(cx, window, |window, cx| window.press("enter", cx));
        assert_eq!(
            cx.update(|cx| host.read(cx).other.clone()),
            ["agent mode", "command review main"]
        );
    }

    #[gpui_kit::test]
    fn slash_jig_runs_a_jig_on_the_agent(cx: &mut TestAppContext) {
        let (window, host) = open_agent(cx);
        step(cx, window, |window, cx| window.input("/j", cx));
        // Enter on "/jig" lists the jigs.
        step(cx, window, |window, cx| window.press("enter", cx));
        assert_eq!(query(cx, &host), "/jig ");
        assert!(events(cx, &host).is_empty());
        step(cx, window, |window, cx| window.input("docs: terse", cx));
        step(cx, window, |window, cx| window.press("enter", cx));
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name.as_deref(), Some("Add docs"));
        assert_eq!(invocation.comment.as_deref(), Some("terse"));
        assert!(invocation.agent);
    }

    #[gpui_kit::test]
    fn an_unknown_slash_command_goes_to_the_agent_as_text(cx: &mut TestAppContext) {
        let (window, host) = open_agent(cx);
        step(cx, window, |window, cx| {
            window.input("/nothing like it", cx)
        });
        assert_eq!(
            rows(cx, &host),
            [super::Row::Custom("/nothing like it".into())]
        );
    }

    #[gpui_kit::test]
    fn right_completes_the_jig_chosen_with_the_arrows(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| window.input("add", cx));
        step(cx, window, |window, cx| window.press("down", cx));
        step(cx, window, |window, cx| window.press("right", cx));
        assert_eq!(query(cx, &host), "add tests");
    }

    #[gpui_kit::test]
    fn right_moves_the_cursor_when_there_is_nothing_to_complete(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        // "xpl" only loosely matches Explain, so it stays a prompt.
        step(cx, window, |window, cx| window.input("xpl", cx));
        step(cx, window, |window, cx| window.press("right", cx));
        assert_eq!(query(cx, &host), "xpl");
        // Away from the end, → just moves the cursor.
        step(cx, window, |window, cx| {
            window.press("backspace", cx);
            window.press("backspace", cx);
            window.press("backspace", cx);
        });
        step(cx, window, |window, cx| window.input("expl", cx));
        step(cx, window, |window, cx| {
            window.press("left", cx);
            window.press("right", cx);
        });
        assert_eq!(query(cx, &host), "expl");
    }

    #[gpui_kit::test]
    fn a_colon_adds_a_note_to_any_command(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, true, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("simplify: keep the early return", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name.as_deref(), Some("Simplify"));
        assert_eq!(invocation.comment.as_deref(), Some("keep the early return"));
    }

    #[gpui_kit::test]
    fn a_colon_note_answers_a_command_that_asks_for_one(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, false, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("controller: Invoice", cx);
            window.press("enter", cx);
        });
        assert!(!in_note_step(cx, &host), "the note is already there");
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.comment.as_deref(), Some("Invoice"));
    }

    #[gpui_kit::test]
    fn a_colon_without_a_matching_command_is_just_text(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, true, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("zzz: qqq", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name, None);
        assert_eq!(invocation.instruction, "zzz: qqq");
    }

    #[gpui_kit::test]
    fn escape_and_backspace_return_to_the_list(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, false, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("controller", cx);
            window.press("enter", cx);
        });
        step(cx, window, |window, cx| window.press("escape", cx));
        assert!(!in_note_step(cx, &host));
        assert!(
            events(cx, &host).is_empty(),
            "Esc in the note step doesn't dismiss"
        );
        assert_eq!(
            cx.update(|cx| host.read(cx).palette.read(cx).query(cx)),
            "controller",
            "the query is restored"
        );

        step(cx, window, |window, cx| window.press("enter", cx));
        assert!(in_note_step(cx, &host));
        step(cx, window, |window, cx| window.press("backspace", cx));
        assert!(!in_note_step(cx, &host));

        step(cx, window, |window, cx| window.press("escape", cx));
        assert_eq!(events(cx, &host), vec![None], "Esc in the list dismisses");
    }

    #[gpui_kit::test]
    fn tab_switches_lanes(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("tab", cx);
            window.press("tab", cx);
        });
        cx.update(|cx| assert!(!host.read(cx).palette.read(cx).agent_lane()));
        step(cx, window, |window, cx| window.press("tab", cx));
        cx.update(|cx| {
            assert!(host.read(cx).palette.read(cx).agent_lane());
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("rename things", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert!(invocation.agent);
        assert_eq!(invocation.instruction, "rename things");
    }

    #[gpui_kit::test]
    fn a_docked_agent_keeps_the_switch_on_jig(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        cx.update(|cx| {
            let palette = host.read(cx).palette.clone();
            palette.update(cx, |palette, _| palette.agent_docked = true);
        });
        step(cx, window, |window, cx| window.press("tab", cx));
        cx.update(|cx| assert!(!host.read(cx).palette.read(cx).agent_lane()));
        // Picking an earlier conversation still works.
        let palette = cx.update(|cx| host.read(cx).palette.clone());
        step(cx, window, move |window, cx| {
            palette.update(cx, |palette, cx| palette.switch_to_agent(window, cx))
        });
        cx.update(|cx| assert!(host.read(cx).palette.read(cx).agent_lane()));
    }

    #[gpui_kit::test]
    fn an_agent_command_shows_the_agent_lane(cx: &mut TestAppContext) {
        let presets =
            presets::parse("[[jig]]\nname = \"Refactor\"\nprompt = \"p\"\nagent = true\n").unwrap();
        let (window, host) = open_with(cx, false, presets);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("refactor", cx);
        });
        cx.update(|cx| {
            let palette = host.read(cx).palette.read(cx);
            assert!(!palette.agent, "the switch itself stays put");
            assert!(palette.agent_lane(), "but the command runs on the agent");
        });
        step(cx, window, |window, cx| window.press("enter", cx));
        assert!(events(cx, &host)[0].clone().unwrap().agent);
    }
}
