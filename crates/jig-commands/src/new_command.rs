//! The floating form for adding a preset command.
//!
//! Tab moves between the fields, Cmd+1/2/3 picks the scope and Cmd+4/5/6
//! whether the command asks for a note (a click works too), Cmd+Enter saves
//! and Esc cancels.

use gpui_kit::component::input::{
    Enter, Escape, IndentInline, Input, InputState, OutdentInline, Textarea, TextareaState,
};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::presets::{CommentMode, Preset, Scope};

actions!(
    jig_new_command,
    [
        ScopeSelection,
        ScopeCursor,
        ScopeFile,
        NoteNone,
        NoteOptional,
        NoteRequired
    ]
);

const CONTEXT: &str = "JigNewCommand";

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("secondary-1", ScopeSelection, Some(CONTEXT)),
        KeyBinding::new("secondary-2", ScopeCursor, Some(CONTEXT)),
        KeyBinding::new("secondary-3", ScopeFile, Some(CONTEXT)),
        KeyBinding::new("secondary-4", NoteNone, Some(CONTEXT)),
        KeyBinding::new("secondary-5", NoteOptional, Some(CONTEXT)),
        KeyBinding::new("secondary-6", NoteRequired, Some(CONTEXT)),
    ]
}

pub enum NewCommandEvent {
    Save(Preset),
    Cancel,
}

pub struct NewCommandForm {
    name: Entity<InputState>,
    prompt: Entity<TextareaState>,
    hint: Entity<InputState>,
    scope: Scope,
    comment: CommentMode,
    /// Runs on the agent rather than as a quick command.
    agent: bool,
    error: Option<SharedString>,
}

impl EventEmitter<NewCommandEvent> for NewCommandForm {}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Name,
    Prompt,
    Hint,
}

impl NewCommandForm {
    /// `prompt` pre-fills the prompt, e.g. from a typed custom command.
    pub fn new(
        prompt: Option<String>,
        scope: Scope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name =
            cx.new(|cx| InputState::new(window, cx).placeholder("Name, e.g. Create controller"));
        let prompt = cx.new(|cx| {
            let state = TextareaState::new(window, cx).placeholder("What should the jig do?");
            match prompt {
                Some(text) => state.default_value(text),
                None => state,
            }
        });
        let hint = cx.new(|cx| {
            InputState::new(window, cx).placeholder("Placeholder for the note, e.g. Entity name")
        });
        name.update(cx, |name, cx| name.focus(window, cx));
        Self {
            name,
            prompt,
            hint,
            scope,
            comment: CommentMode::None,
            agent: false,
            error: None,
        }
    }

    /// Show why saving failed, e.g. a duplicate name.
    pub fn set_error(&mut self, error: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.error = Some(error.into());
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let name = self.name.read(cx).value().trim().to_string();
        let prompt = self.prompt.read(cx).value().trim().to_string();
        let hint = self.hint.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.set_error("Give the jig a name.", cx);
        } else if prompt.is_empty() {
            self.set_error("Describe what the jig should do.", cx);
        } else {
            let asks = self.comment != CommentMode::None;
            cx.emit(NewCommandEvent::Save(Preset {
                name,
                scope: self.scope,
                prompt,
                comment: self.comment,
                agent: self.agent,
                comment_hint: (asks && !hint.is_empty()).then_some(hint),
            }));
        }
    }

    fn fields(&self) -> Vec<Field> {
        let mut fields = vec![Field::Name, Field::Prompt];
        if self.comment != CommentMode::None {
            fields.push(Field::Hint);
        }
        fields
    }

    fn focused(&self, window: &Window, cx: &App) -> Option<Field> {
        if self.name.focus_handle(cx).is_focused(window) {
            Some(Field::Name)
        } else if self.prompt.focus_handle(cx).is_focused(window) {
            Some(Field::Prompt)
        } else if self.hint.focus_handle(cx).is_focused(window) {
            Some(Field::Hint)
        } else {
            None
        }
    }

    /// Move focus `step` fields along, wrapping around.
    fn cycle(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let fields = self.fields();
        let current = self
            .focused(window, cx)
            .and_then(|field| fields.iter().position(|f| *f == field))
            .unwrap_or(0) as isize;
        let next = fields[(current + step).rem_euclid(fields.len() as isize) as usize];
        match next {
            Field::Name => self.name.update(cx, |input, cx| input.focus(window, cx)),
            Field::Prompt => self.prompt.update(cx, |input, cx| input.focus(window, cx)),
            Field::Hint => self.hint.update(cx, |input, cx| input.focus(window, cx)),
        }
    }

    fn on_enter(&mut self, action: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if action.secondary {
            cx.stop_propagation();
            self.save(cx);
        } else if self.focused(window, cx) == Some(Field::Name) {
            // Enter in the one-line name field moves on to the prompt.
            self.cycle(1, window, cx);
        }
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        cx.emit(NewCommandEvent::Cancel);
    }

    fn on_tab(&mut self, _: &IndentInline, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle(1, window, cx);
    }

    fn on_shift_tab(&mut self, _: &OutdentInline, window: &mut Window, cx: &mut Context<Self>) {
        self.cycle(-1, window, cx);
    }

    fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        self.scope = scope;
        cx.notify();
    }

    fn set_agent(&mut self, agent: bool, cx: &mut Context<Self>) {
        self.agent = agent;
        cx.notify();
    }

    fn set_comment(&mut self, comment: CommentMode, cx: &mut Context<Self>) {
        self.comment = comment;
        cx.notify();
    }
}

/// A selectable option with its shortcut, e.g. "cursor ⌘2".
fn chip(
    id: (&'static str, usize),
    label: &'static str,
    shortcut: String,
    active: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    h_flex()
        .id(id)
        .h(px(26.))
        .px_2p5()
        .gap_1p5()
        .rounded(px(7.))
        .text_size(px(12.))
        // Like a segmented control: the chosen option is filled with the accent.
        .when(active, |chip| {
            chip.bg(theme.primary).text_color(theme.primary_foreground)
        })
        .when(!active, |chip| {
            chip.bg(theme.foreground.opacity(0.06))
                .hover(|chip| chip.bg(theme.foreground.opacity(0.1)))
        })
        .child(label)
        .child(
            div()
                .text_size(px(11.))
                .when(active, |this| {
                    this.text_color(theme.primary_foreground.opacity(0.75))
                })
                .when(!active, |this| this.text_color(theme.muted_foreground))
                .child(shortcut),
        )
}

impl Render for NewCommandForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let scopes: Vec<_> = Scope::ALL
            .iter()
            .enumerate()
            .map(|(index, &scope)| {
                let label = format!("⌘{}", index + 1);
                chip(
                    ("jig-scope", index),
                    scope.label(),
                    label,
                    scope == self.scope,
                    cx,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| this.set_scope(scope, cx)),
                )
            })
            .collect();
        let notes: Vec<_> = CommentMode::ALL
            .iter()
            .enumerate()
            .map(|(index, &mode)| {
                let label = format!("⌘{}", index + 4);
                chip(
                    ("jig-note", index),
                    mode.label(),
                    label,
                    mode == self.comment,
                    cx,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| this.set_comment(mode, cx)),
                )
            })
            .collect();
        let lanes: Vec<_> = [false, true]
            .into_iter()
            .enumerate()
            .map(|(ix, agent)| {
                chip(
                    ("jig-lane", ix),
                    crate::surface::lane_name(agent),
                    String::new(),
                    agent == self.agent,
                    cx,
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| this.set_agent(agent, cx)),
                )
            })
            .collect();
        let focused = self.focused(window, cx);
        let theme = cx.theme();
        let label = |text: &'static str| {
            div()
                .pt_1()
                .text_size(px(11.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.muted_foreground)
                .child(text)
        };

        let form = crate::surface::panel(cx)
            .flex()
            .flex_col()
            .key_context(CONTEXT)
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_tab))
            .capture_action(cx.listener(Self::on_shift_tab))
            .on_action(
                cx.listener(|this, _: &ScopeSelection, _, cx| this.set_scope(Scope::Selection, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ScopeCursor, _, cx| this.set_scope(Scope::Cursor, cx)),
            )
            .on_action(cx.listener(|this, _: &ScopeFile, _, cx| this.set_scope(Scope::File, cx)))
            .on_action(
                cx.listener(|this, _: &NoteNone, _, cx| this.set_comment(CommentMode::None, cx)),
            )
            .on_action(cx.listener(|this, _: &NoteOptional, _, cx| {
                this.set_comment(CommentMode::Optional, cx)
            }))
            .on_action(cx.listener(|this, _: &NoteRequired, _, cx| {
                this.set_comment(CommentMode::Required, cx)
            }))
            .w(px(460.))
            .p_4()
            .gap_2()
            .text_size(px(13.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("New Jig"),
            )
            .child(label("Name"))
            .child(Input::new(&self.name))
            .child(label("Works on"))
            .child(h_flex().gap_1p5().children(scopes))
            .child(label("Prompt"))
            .child(Textarea::new(&self.prompt).h(px(120.)))
            .child(label("Asks for a note"))
            .child(h_flex().gap_1p5().children(notes))
            .when(self.comment != CommentMode::None, |this| {
                this.child(Input::new(&self.hint))
            })
            .child(label("Runs on"))
            .child(
                h_flex().gap_1p5().children(lanes).child(
                    div()
                        .pl_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(if self.agent {
                            "Works across the project; you review every edit."
                        } else {
                            "One quick edit; sees this file and AGENTS.md."
                        }),
                ),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_xs().text_color(theme.danger).child(error))
            })
            .child(div().text_xs().text_color(theme.muted_foreground).child(
                if focused == Some(Field::Name) {
                    "tab to the prompt · ⌘↩ save · esc cancel"
                } else {
                    "⌘↩ save · esc cancel"
                },
            ));
        crate::motion::pop_in(form, "jig-new-command")
    }
}
