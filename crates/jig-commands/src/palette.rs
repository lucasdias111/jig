//! The floating input that opens beside the code: a message for the
//! agent, listed above the earlier conversations it matches. "/" lists the
//! agent's commands and skills. Enter runs the highlighted row; Esc, or
//! clicking away, dismisses it.

use gpui_kit::component::input::{
    Backspace, Enter, Escape, InputEvent, MoveDown, MoveEnd, MoveRight, MoveUp, Textarea,
    TextareaState,
};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

const MAX_ROWS: usize = 8;
/// How tall a message grows, in lines, before it scrolls.
const MAX_MESSAGE_LINES: usize = 8;
/// The palette's key context, for its own shortcuts.
pub const CONTEXT: &str = "JigPalette";
/// How wide the input is, to find it room beside the code.
pub const WIDTH: f32 = 420.;

actions!(jig_palette, [PickModel]);

pub enum PaletteEvent {
    /// Start a conversation with this message.
    Run(String),
    /// Pick up the earlier conversation with this id.
    Resume(String),
    /// Run one of the agent's commands or skills in a new conversation.
    RunAgentCommand {
        name: String,
        arguments: String,
    },
    /// An agent and model picked for the window. The palette stays open.
    PickModel(ModelChoice),
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

/// An agent with one of its models, as the chip offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelChoice {
    /// The agent's key.
    pub agent: Option<String>,
    /// What it's listed under: the agent, e.g. "Claude Code".
    pub group: String,
    /// The model alone, e.g. "Opus 5.5".
    pub name: String,
    /// `provider/model` for OpenCode, the agent's own name for other
    /// agents; `None` for the agent's default.
    pub model: Option<String>,
    /// As listed, e.g. "Claude Code · Opus 5.5".
    pub label: String,
}

/// What the chip offers, and which is in use.
#[derive(Clone, Debug, Default)]
pub struct ModelMenu {
    pub choices: Vec<ModelChoice>,
    pub current: Option<usize>,
}

impl ModelMenu {
    fn current(&self) -> Option<&ModelChoice> {
        self.choices.get(self.current?)
    }

    /// The agents, in the order they're listed.
    fn groups(&self) -> Vec<&str> {
        let mut groups: Vec<&str> = Vec::new();
        for choice in &self.choices {
            if !groups.contains(&choice.group.as_str()) {
                groups.push(&choice.group);
            }
        }
        groups
    }
}

/// Picking a model: first the agent, then its models.
struct Picking {
    /// The list query to restore after.
    query: String,
    /// The agent chosen, once one is.
    group: Option<String>,
}

/// One of the agent's commands or skills, run with "/name".
#[derive(Clone, Debug, PartialEq)]
pub struct AgentCommandInfo {
    pub name: String,
    pub description: String,
    pub skill: bool,
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    /// The typed text, as a message for the agent.
    Message(String),
    Conversation(usize),
    Command(usize),
    /// An agent to pick models from, by its place in the list.
    Group(usize),
    /// A model to pick, while picking one.
    Model(usize),
}

impl Row {
    fn section(&self) -> &'static str {
        match self {
            Row::Message(_) => "Message",
            Row::Conversation(_) => "Conversations",
            Row::Command(_) => "Commands",
            Row::Group(_) => "Agents",
            Row::Model(_) => "Models",
        }
    }
}

pub struct CommandPalette {
    /// Wraps and grows; Shift-Enter starts a new line.
    input: Entity<TextareaState>,
    rows: Vec<Row>,
    /// The query `rows` were built from.
    filtered_query: String,
    selected: usize,
    has_selection: bool,
    conversations: Vec<PastConversation>,
    commands: Vec<AgentCommandInfo>,
    /// The chip's agents and models.
    models: ModelMenu,
    /// Picking a model.
    picking: Option<Picking>,
    _subscription: Subscription,
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

impl Focusable for CommandPalette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl CommandPalette {
    /// `has_selection` decides whether the message is said to be about the
    /// selection.
    pub fn new(has_selection: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(Self::list_placeholder(has_selection))
                .auto_grow(1, MAX_MESSAGE_LINES)
                .soft_wrap(true)
        });
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => this.refilter(cx),
                    InputEvent::Blur => cx.emit(PaletteEvent::Dismissed),
                    _ => {}
                },
            );
        input.update(cx, |input, cx| input.focus(window, cx));
        let mut this = Self {
            input,
            rows: Vec::new(),
            filtered_query: String::new(),
            selected: 0,
            has_selection,
            conversations: Vec::new(),
            commands: Vec::new(),
            models: ModelMenu::default(),
            picking: None,
            _subscription: subscription,
        };
        this.refilter(cx);
        this
    }

    fn list_placeholder(has_selection: bool) -> &'static str {
        if has_selection {
            "Ask the agent about the selection, or / for commands…"
        } else {
            "Ask the agent, or / for commands…"
        }
    }

    /// What the model chip offers.
    pub fn with_models(mut self, models: ModelMenu) -> Self {
        self.models = models;
        self
    }

    /// New lists for the chip, e.g. once an agent's models arrive.
    pub fn set_models(&mut self, models: ModelMenu, cx: &mut Context<Self>) {
        self.models = models;
        if self.picking.is_some() {
            self.refilter(cx);
        }
        cx.notify();
    }

    fn models(&self) -> &ModelMenu {
        &self.models
    }

    /// ⌥M or the chip: list the agents in place of the conversations, then
    /// the one picked's models. With only one, its models.
    fn start_picking(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picking.is_some() || self.models().choices.is_empty() {
            return;
        }
        let groups = self.models().groups();
        let group = (groups.len() == 1).then(|| groups[0].to_string());
        self.picking = Some(Picking {
            query: self.query(cx),
            group,
        });
        self.show_picking_step(window, cx);
    }

    /// The step on screen, with the input emptied and the one in use
    /// highlighted.
    fn show_picking_step(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picking) = &self.picking else {
            return;
        };
        let placeholder = match &picking.group {
            Some(group) => format!("Pick a {group} model…"),
            None => "Pick an agent, or type a model…".to_string(),
        };
        self.input.update(cx, |input, cx| {
            set_text(input, "", window, cx);
            input.set_placeholder(placeholder, window, cx);
        });
        self.refilter(cx);
        let in_use = self.models().current.and_then(|current| {
            let choice = &self.models().choices[current];
            let groups = self.models().groups();
            Some(match &self.picking.as_ref()?.group {
                Some(_) => Row::Model(current),
                None => Row::Group(groups.iter().position(|group| *group == choice.group)?),
            })
        });
        if let Some(at) = in_use.and_then(|row| self.rows.iter().position(|r| *r == row)) {
            self.selected = at;
        }
        cx.notify();
    }

    /// Back from the models to the agents, if there are several. False
    /// when there's no step to go back to.
    fn picking_back(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let several = self.models().groups().len() > 1;
        match self.picking.as_mut() {
            Some(picking) if picking.group.is_some() && several => {
                picking.group = None;
                self.show_picking_step(window, cx);
                true
            }
            _ => false,
        }
    }

    fn stop_picking(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(picking) = self.picking.take() else {
            return;
        };
        let placeholder = Self::list_placeholder(self.has_selection);
        self.input.update(cx, |input, cx| {
            set_text(input, picking.query, window, cx);
            input.set_placeholder(placeholder, window, cx);
        });
        self.refilter(cx);
    }

    fn on_pick_model(&mut self, _: &PickModel, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.picking.is_some() {
            self.stop_picking(window, cx);
        } else {
            self.start_picking(window, cx);
        }
    }

    /// Picking: the agents whose name has the typed words,
    /// then the models that do, so a model can be typed straight away; or,
    /// once one is picked, its models.
    fn filter_models(&mut self, query: &str) {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let has = |text: &str| {
            let text = text.to_lowercase();
            words.iter().all(|word| text.contains(word))
        };
        let group = self
            .picking
            .as_ref()
            .and_then(|picking| picking.group.clone());
        let menu = self.models();
        self.rows = match group {
            Some(group) => menu
                .choices
                .iter()
                .enumerate()
                .filter(|(_, choice)| choice.group == group && has(&choice.name))
                .map(|(index, _)| Row::Model(index))
                .collect(),
            None => {
                let groups: Vec<Row> = menu
                    .groups()
                    .iter()
                    .enumerate()
                    .filter(|(_, group)| has(group))
                    .map(|(index, _)| Row::Group(index))
                    .collect();
                let models = menu
                    .choices
                    .iter()
                    .enumerate()
                    .filter(|(_, choice)| !words.is_empty() && has(&choice.label))
                    .map(|(index, _)| Row::Model(index));
                groups.into_iter().chain(models).collect()
            }
        };
        self.selected = 0;
    }

    /// The chip at the footer's end: the agent and model in use, which ⌥M
    /// or a click changes for the window.
    fn render_chip(&self, accent: Hsla, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let menu = self.models();
        if menu.choices.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let label = menu
            .current()
            .map_or("Pick a model".to_string(), |choice| choice.label.clone());
        Some(
            h_flex()
                .id("jig-model-chip")
                .debug_selector(|| "palette-model-chip".into())
                .flex_none()
                .max_w(px(220.))
                .gap_1()
                .px_1p5()
                .rounded(px(5.))
                .text_size(px(11.))
                .text_color(theme.muted_foreground)
                .cursor_pointer()
                .hover(|this| this.bg(theme.foreground.opacity(0.06)).text_color(accent))
                .when(self.picking.is_some(), |this| {
                    this.bg(accent.opacity(0.14)).text_color(accent)
                })
                .child(div().truncate().child(label))
                .child(div().flex_none().child("⌥M"))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        if this.picking.is_some() {
                            this.stop_picking(window, cx);
                        } else {
                            this.start_picking(window, cx);
                        }
                    }),
                ),
        )
    }

    /// The earlier conversations and the agent's commands, listed once
    /// they've been fetched.
    pub fn set_agent_catalog(
        &mut self,
        conversations: Vec<PastConversation>,
        commands: Vec<AgentCommandInfo>,
        cx: &mut Context<Self>,
    ) {
        self.conversations = conversations;
        self.commands = commands;
        let selected = self.rows.get(self.selected).cloned();
        self.refilter(cx);
        // Keep the arrows' choice when the list fills in under it.
        if let Some(index) = selected.and_then(|row| self.rows.iter().position(|r| *r == row)) {
            self.selected = index;
        }
    }

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        if self.picking.is_some() {
            self.filter_models(&query);
        } else if query.trim_start().starts_with('/') {
            self.filter_commands(&query);
        } else {
            self.filter_conversations(&query);
        }
        self.filtered_query = query;
        cx.notify();
    }

    /// The typed message, then the earlier conversations it matches (all
    /// of them before anything is typed).
    fn filter_conversations(&mut self, query: &str) {
        let query = query.trim();
        let lower = query.to_lowercase();
        self.rows = Vec::new();
        if !query.is_empty() {
            self.rows.push(Row::Message(query.to_string()));
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

    /// After "/": the commands and skills whose names match. Text after
    /// the name is the command's arguments. With nothing matching, the text
    /// goes to the agent as it is.
    fn filter_commands(&mut self, query: &str) {
        let typed = query.trim_start().trim_start_matches('/');
        let (name, arguments) = typed.split_once(' ').unwrap_or((typed, ""));
        let name = name.to_lowercase();
        let mut matches: Vec<(u32, usize)> = self
            .commands
            .iter()
            .enumerate()
            .filter_map(|(index, command)| {
                let own = command.name.to_lowercase();
                if arguments.is_empty() {
                    score(&own, &name).map(|score| (score, index))
                } else {
                    (own == name).then_some((0, index))
                }
            })
            .collect();
        matches.sort();
        self.rows = matches
            .into_iter()
            .map(|(_, index)| Row::Command(index))
            .collect();
        if self.rows.is_empty() {
            self.rows.push(Row::Message(query.trim().to_string()));
        }
        self.selected = 0;
    }

    /// The rest of the highlighted command's name when the text typed so
    /// far is how it starts and the cursor is at its end: "iew" after "/rev".
    fn completion(&self, cx: &App) -> Option<String> {
        let input = self.input.read(cx);
        let query = input.value();
        let at_end = input.selected_range() == (query.len()..query.len());
        if !at_end {
            return None;
        }
        let Some(Row::Command(index)) = self.rows.get(self.selected) else {
            return None;
        };
        let typed = query.trim_start().strip_prefix('/')?;
        if typed.contains(' ') {
            return None;
        }
        completion(&self.commands[*index].name, typed).map(str::to_string)
    }

    /// → or End at the end of the text completes the highlighted command's
    /// name; anywhere else they move the cursor as usual.
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

    /// Text typed in the same frame as Enter hasn't been filtered yet; do
    /// it now so the right row is chosen.
    fn ensure_filtered(&mut self, cx: &mut Context<Self>) {
        if self.query(cx) != self.filtered_query {
            self.refilter(cx);
        }
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.rows.get(index) {
            Some(Row::Message(text)) => cx.emit(PaletteEvent::Run(text.clone())),
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
            Some(Row::Group(index)) => {
                let group = self.models().groups()[*index].to_string();
                if let Some(picking) = self.picking.as_mut() {
                    picking.group = Some(group);
                }
                self.show_picking_step(window, cx);
            }
            Some(Row::Model(index)) => {
                let index = *index;
                let choice = self.models.choices[index].clone();
                self.models.current = Some(index);
                self.stop_picking(window, cx);
                cx.emit(PaletteEvent::PickModel(choice));
            }
            None => {}
        }
    }

    fn on_enter(&mut self, action: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        // Shift-Enter in a message starts a new line.
        if action.shift && self.picking.is_none() {
            return;
        }
        cx.stop_propagation();
        self.ensure_filtered(cx);
        self.choose(self.selected, window, cx);
    }

    /// Backspace in an empty search goes back a step while picking a model.
    fn on_backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.picking.is_some() && self.query(cx).is_empty() {
            cx.stop_propagation();
            if !self.picking_back(window, cx) {
                self.stop_picking(window, cx);
            }
        }
    }

    fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.picking.is_some() {
            self.stop_picking(window, cx);
        } else {
            cx.emit(PaletteEvent::Dismissed);
        }
    }

    fn on_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if !self.rows.is_empty() {
            self.selected = (self.selected + self.rows.len() - 1) % self.rows.len();
            cx.notify();
        }
    }

    fn on_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if !self.rows.is_empty() {
            self.selected = (self.selected + 1) % self.rows.len();
            cx.notify();
        }
    }

    /// Spotlight-sized input, marked with the agent's icon, with the rest
    /// of a command's name suggested after the typed text.
    fn render_input(&self, accent: Hsla, cx: &mut Context<Self>) -> impl IntoElement {
        let completion = self.completion(cx);
        let muted = cx.theme().muted_foreground;
        h_flex()
            .items_start()
            .gap_1()
            .px_1()
            .text_size(px(15.))
            .child(
                div().h(px(32.)).flex().items_center().child(
                    crate::surface::agent_icon()
                        .size(px(15.))
                        .text_color(accent),
                ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .child(Textarea::new(&self.input).appearance(false))
                    .children(
                        completion
                            .as_ref()
                            .map(|rest| suggestion(self.input.clone(), rest.clone(), muted)),
                    ),
            )
            .when(completion.is_some(), |this| {
                this.child(
                    div()
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .child(crate::surface::hint("→ complete", cx)),
                )
            })
    }

    fn render_row(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = index == self.selected;
        let (label, detail): (SharedString, SharedString) = match row {
            Row::Message(text) => (format!("“{text}”").into(), "".into()),
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
            Row::Group(index) => {
                let menu = self.models();
                let group = menu.groups()[*index];
                let in_use = menu.current().is_some_and(|choice| choice.group == group);
                let count = menu
                    .choices
                    .iter()
                    .filter(|choice| choice.group == group)
                    .count();
                let detail = match (in_use, count) {
                    (true, _) => "in use".to_string(),
                    (false, 1) => "1 model".to_string(),
                    (false, count) => format!("{count} models"),
                };
                (group.to_string().into(), detail.into())
            }
            Row::Model(index) => {
                let menu = self.models();
                let choice = &menu.choices[*index];
                // Alone under its agent; in full among search results.
                let within = self
                    .picking
                    .as_ref()
                    .is_some_and(|picking| picking.group.is_some());
                let label = if within { &choice.name } else { &choice.label };
                (
                    label.clone().into(),
                    if menu.current == Some(*index) {
                        "in use".into()
                    } else {
                        "".into()
                    },
                )
            }
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
        let accent = crate::surface::agent_accent(cx);
        let theme = cx.theme();
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
                            .when(description.is_some(), |this| this.flex_none())
                            .child(label),
                    )
                    .children(description),
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
}

impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let first = self.selected.saturating_sub(MAX_ROWS - 1);
        let rows: Vec<_> = self
            .rows
            .clone()
            .iter()
            .enumerate()
            .skip(first)
            .take(MAX_ROWS)
            .flat_map(|(index, row)| {
                // Label each kind of row where it starts: the message, and
                // the commands or conversations below.
                let section = (index == first || self.rows[index - 1].section() != row.section())
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
            .collect();
        let accent = crate::surface::agent_accent(cx);
        let input = self.render_input(accent, cx).into_any_element();
        let chip = self
            .render_chip(accent, cx)
            .map(IntoElement::into_any_element);
        let theme = cx.theme();
        let footer = if let Some(picking) = &self.picking {
            match (&picking.group, self.rows.get(self.selected)) {
                (None, Some(Row::Group(_))) => "↩ its models · esc back".to_string(),
                (Some(_), _) if self.models().groups().len() > 1 => {
                    "↩ use it in this window · ⌫ back · esc done".to_string()
                }
                _ => "↩ use it in this window · esc back".to_string(),
            }
        } else {
            match self.rows.get(self.selected) {
                Some(Row::Conversation(_)) => "Picks up this conversation where it left off".into(),
                Some(Row::Command(_)) => {
                    "Runs in a new conversation · you review every edit".to_string()
                }
                _ => "Works across the project · you review every edit".to_string(),
            }
        };

        let palette = crate::surface::panel(cx)
            .flex()
            .flex_col()
            .key_context(CONTEXT)
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_up))
            .capture_action(cx.listener(Self::on_down))
            .capture_action(cx.listener(Self::on_pick_model))
            .capture_action(cx.listener(Self::on_right))
            .capture_action(cx.listener(Self::on_end))
            .capture_action(cx.listener(Self::on_backspace))
            .w(px(WIDTH))
            .p_1p5()
            .gap_0p5()
            .border_color(accent.opacity(0.55))
            .child(input)
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
                h_flex()
                    .px_2p5()
                    .pt_1()
                    .pb_0p5()
                    .gap_2()
                    .text_size(px(11.))
                    .text_color(accent)
                    .child(div().flex_1().min_w_0().truncate().child(footer))
                    .children(chip),
            );
        crate::motion::pop_in(palette, "jig-palette")
    }
}

/// The rest of `name` after `typed`, when `typed` is how it starts, in any
/// case; `None` when it isn't, or there's nothing left.
fn completion<'a>(name: &'a str, typed: &str) -> Option<&'a str> {
    let mut rest = name.char_indices();
    for wanted in typed.chars() {
        let (_, got) = rest.next()?;
        if !got.to_lowercase().eq(wanted.to_lowercase()) {
            return None;
        }
    }
    let (at, _) = rest.next()?;
    (!typed.is_empty()).then(|| &name[at..])
}

/// Lower is better: prefix, then substring, then a subsequence ranked by how
/// spread out its characters are. `None` when `query` isn't a subsequence.
fn score(name: &str, query: &str) -> Option<u32> {
    if name.starts_with(query) {
        return Some(0);
    }
    if name.contains(query) {
        return Some(1);
    }
    let mut gaps = 0;
    let mut chars = name.chars();
    for wanted in query.chars() {
        let mut skipped = 0;
        loop {
            match chars.next() {
                Some(c) if c == wanted => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
        gaps += skipped;
    }
    Some(2 + gaps)
}

/// Replace the text with the cursor after it, where typing goes on. A
/// textarea's own `set_value` puts it at the start.
fn set_text(
    input: &mut TextareaState,
    text: impl Into<String>,
    window: &mut Window,
    cx: &mut Context<TextareaState>,
) {
    let text = text.into();
    let end = text.len();
    input.set_value(text, window, cx);
    input.set_selected_range(end..end, cx);
}

/// The rest of a command's name in grey after the typed text. Painted after the
/// input, so it goes where the input has just laid its text out, in the same
/// font at the size the input actually drew.
fn suggestion(input: Entity<TextareaState>, rest: String, color: Hsla) -> impl IntoElement {
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
    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Point, Render, Styled,
        Subscription, TestAppContext, Window, WindowBounds, WindowOptions, div, px, size,
    };

    use super::{CommandPalette, PaletteEvent, Row};

    struct Host {
        palette: Entity<CommandPalette>,
        /// What it said: "run …", "resume ses_1", "command review main".
        events: Vec<String>,
        _subscription: Subscription,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.palette.clone())
        }
    }

    /// The palette with two earlier conversations and two commands listed.
    fn open(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, Entity<Host>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            // Jig binds it from its shortcuts table.
            cx.bind_keys([gpui_kit::KeyBinding::new(
                "alt-m",
                super::PickModel,
                Some(super::CONTEXT),
            )]);
        });
        let (window, host) = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(600.), px(400.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let palette = cx.new(|cx| CommandPalette::new(true, window, cx));
                    let subscription =
                        cx.subscribe(&palette, |host: &mut Host, _, event: &PaletteEvent, _| {
                            host.events.push(match event {
                                PaletteEvent::Run(text) => format!("run {text}"),
                                PaletteEvent::Resume(id) => format!("resume {id}"),
                                PaletteEvent::RunAgentCommand { name, arguments } => {
                                    format!("command {name} {arguments}").trim().into()
                                }
                                PaletteEvent::PickModel(choice) => {
                                    format!("model {}", choice.label)
                                }
                                PaletteEvent::Dismissed => "dismissed".into(),
                            })
                        });
                    Host {
                        palette,
                        events: Vec::new(),
                        _subscription: subscription,
                    }
                })
            })
            .unwrap()
        });
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

    fn step(
        cx: &mut TestAppContext,
        window: gpui_kit::AnyWindowHandle,
        f: impl FnOnce(&mut Window, &mut gpui_kit::App),
    ) {
        cx.update_window(window, |_, window, cx| f(window, cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn events(cx: &mut TestAppContext, host: &Entity<Host>) -> Vec<String> {
        cx.update(|cx| host.read(cx).events.clone())
    }

    fn query(cx: &mut TestAppContext, host: &Entity<Host>) -> String {
        cx.update(|cx| host.read(cx).palette.read(cx).query(cx))
    }

    fn rows(cx: &mut TestAppContext, host: &Entity<Host>) -> Vec<Row> {
        cx.update(|cx| host.read(cx).palette.read(cx).rows.clone())
    }

    #[gpui_kit::test]
    fn typed_text_is_a_message_for_the_agent(cx: &mut TestAppContext) {
        let (window, host) = open(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("make this async", cx);
        });
        step(cx, window, |window, cx| window.press("enter", cx));
        assert_eq!(events(cx, &host), ["run make this async"]);
    }

    #[gpui_kit::test]
    fn escape_dismisses(cx: &mut TestAppContext) {
        let (window, host) = open(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("escape", cx);
        });
        assert_eq!(events(cx, &host), ["dismissed"]);
    }

    #[gpui_kit::test]
    fn earlier_conversations_are_listed_below_the_message(cx: &mut TestAppContext) {
        let (window, host) = open(cx);
        assert_eq!(
            rows(cx, &host),
            [Row::Conversation(0), Row::Conversation(1)]
        );
        step(cx, window, |window, cx| window.input("flaky", cx));
        assert_eq!(
            rows(cx, &host),
            [Row::Message("flaky".into()), Row::Conversation(1)]
        );
        step(cx, window, |window, cx| {
            window.press("down", cx);
            window.press("enter", cx);
        });
        assert_eq!(events(cx, &host), ["resume ses_2"]);
    }

    #[gpui_kit::test]
    fn slash_lists_commands_and_runs_one_with_arguments(cx: &mut TestAppContext) {
        let (window, host) = open(cx);
        step(cx, window, |window, cx| window.input("/", cx));
        assert_eq!(rows(cx, &host), [Row::Command(0), Row::Command(1)]);
        step(cx, window, |window, cx| window.input("rev", cx));
        assert_eq!(rows(cx, &host), [Row::Command(0)]);
        step(cx, window, |window, cx| window.press("right", cx));
        assert_eq!(query(cx, &host), "/review");
        step(cx, window, |window, cx| window.input(" main", cx));
        step(cx, window, |window, cx| window.press("enter", cx));
        assert_eq!(events(cx, &host), ["command review main"]);
    }

    #[gpui_kit::test]
    fn an_unknown_slash_command_goes_to_the_agent_as_text(cx: &mut TestAppContext) {
        let (window, host) = open(cx);
        step(cx, window, |window, cx| {
            window.input("/nothing like it", cx)
        });
        assert_eq!(rows(cx, &host), [Row::Message("/nothing like it".into())]);
    }

    #[gpui_kit::test]
    fn shift_enter_starts_a_new_line(cx: &mut TestAppContext) {
        let (window, host) = open(cx);
        step(cx, window, |window, cx| window.input("rename it", cx));
        step(cx, window, |window, cx| window.press("shift-enter", cx));
        step(cx, window, |window, cx| window.input("and its tests", cx));
        assert_eq!(query(cx, &host), "rename it\nand its tests");
        assert!(events(cx, &host).is_empty());
        step(cx, window, |window, cx| window.press("enter", cx));
        assert_eq!(events(cx, &host), ["run rename it\nand its tests"]);
    }

    #[gpui_kit::test]
    fn right_moves_the_cursor_when_there_is_nothing_to_complete(cx: &mut TestAppContext) {
        let (window, host) = open(cx);
        step(cx, window, |window, cx| window.input("/rev", cx));
        step(cx, window, |window, cx| {
            window.press("left", cx);
            window.press("right", cx);
        });
        assert_eq!(query(cx, &host), "/rev", "away from the end, → just moves");
    }

    #[gpui_kit::test]
    fn the_chip_picks_an_agent_then_its_model(cx: &mut TestAppContext) {
        let (window, host) = open(cx);
        let choice = |agent: &str, group: &str, name: &str| super::ModelChoice {
            agent: Some(agent.into()),
            group: group.into(),
            name: name.into(),
            model: Some(name.to_lowercase()),
            label: format!("{group} · {name}"),
        };
        cx.update(|cx| {
            let palette = host.read(cx).palette.clone();
            palette.update(cx, |palette, _| {
                palette.models = super::ModelMenu {
                    choices: vec![
                        choice("opencode", "OpenCode", "Haiku"),
                        choice("opencode", "OpenCode", "Sonnet"),
                        choice("claude-code", "Claude Code", "Opus"),
                    ],
                    current: Some(0),
                };
            })
        });
        step(cx, window, |window, cx| window.input("explain", cx));
        step(cx, window, |window, cx| window.press("alt-m", cx));
        assert_eq!(query(cx, &host), "", "the typed text is set aside");
        assert_eq!(rows(cx, &host), [Row::Group(0), Row::Group(1)]);
        // Enter on OpenCode lists its models, ⌫ goes back.
        step(cx, window, |window, cx| window.press("enter", cx));
        assert_eq!(rows(cx, &host), [Row::Model(0), Row::Model(1)]);
        step(cx, window, |window, cx| window.press("backspace", cx));
        assert_eq!(rows(cx, &host), [Row::Group(0), Row::Group(1)]);
        // A model can be typed straight away too.
        step(cx, window, |window, cx| window.input("opus", cx));
        assert_eq!(rows(cx, &host), [Row::Model(2)]);
        step(cx, window, |window, cx| window.press("enter", cx));
        assert_eq!(events(cx, &host), ["model Claude Code · Opus"]);
        assert_eq!(query(cx, &host), "explain", "and back after");
        step(cx, window, |window, cx| window.press("alt-m", cx));
        step(cx, window, |window, cx| window.press("escape", cx));
        assert!(cx.update(|cx| host.read(cx).palette.read(cx).picking.is_none()));
    }
}
