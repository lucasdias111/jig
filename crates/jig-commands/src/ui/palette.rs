//! The floating input that opens beside the code: a message for the
//! agent, listed above the earlier conversations it matches. "/" lists the
//! agent's commands and skills. Enter runs the highlighted row; Esc, or
//! clicking away, dismisses it.

use gpui_kit::component::input::{
    Backspace, Enter, Escape, InputEvent, MoveDown, MoveEnd, MoveRight, MoveUp,
    TextareaState,
};
use gpui_kit::*;

use crate::domain::palette::{
    self, AgentCommandInfo, ModelChoice, ModelMenu, PastConversation, Picking, Row,
};

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
    pub fn start_picking(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        self.rows = if let Some(picking) = &self.picking {
            palette::model_rows(&self.models, picking.group.as_deref(), &query)
        } else if query.trim_start().starts_with('/') {
            palette::command_rows(&self.commands, &query)
        } else {
            palette::conversation_rows(&self.conversations, &query)
        };
        self.selected = 0;
        self.filtered_query = query;
        cx.notify();
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
        palette::completion(&self.commands[*index].name, typed).map(str::to_string)
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
                let arguments = palette::command_arguments(&self.query(cx));
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

mod view;

#[cfg(test)]
mod tests;
