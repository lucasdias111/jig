//! The floating form for adding a preset command.
//!
//! Tab moves between Name and Prompt, Cmd+1/2/3 (or a click) picks the
//! scope, Cmd+Enter saves and Esc cancels.

use gpui_kit::component::input::{
    Enter, Escape, IndentInline, Input, InputState, OutdentInline, Textarea, TextareaState,
};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::presets::{Preset, Scope};

actions!(jig_new_command, [ScopeSelection, ScopeCursor, ScopeFile]);

const CONTEXT: &str = "JigNewCommand";

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("secondary-1", ScopeSelection, Some(CONTEXT)),
        KeyBinding::new("secondary-2", ScopeCursor, Some(CONTEXT)),
        KeyBinding::new("secondary-3", ScopeFile, Some(CONTEXT)),
    ]
}

pub enum NewCommandEvent {
    Save(Preset),
    Cancel,
}

pub struct NewCommandForm {
    name: Entity<InputState>,
    prompt: Entity<TextareaState>,
    scope: Scope,
    error: Option<SharedString>,
}

impl EventEmitter<NewCommandEvent> for NewCommandForm {}

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
            let state = TextareaState::new(window, cx).placeholder("What should the command do?");
            match prompt {
                Some(text) => state.default_value(text),
                None => state,
            }
        });
        name.update(cx, |name, cx| name.focus(window, cx));
        Self {
            name,
            prompt,
            scope,
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
        if name.is_empty() {
            self.set_error("Give the command a name.", cx);
        } else if prompt.is_empty() {
            self.set_error("Describe what the command should do.", cx);
        } else {
            cx.emit(NewCommandEvent::Save(Preset {
                name,
                scope: self.scope,
                prompt,
            }));
        }
    }

    fn on_enter(&mut self, action: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if action.secondary {
            cx.stop_propagation();
            self.save(cx);
        } else if self.name.focus_handle(cx).is_focused(window) {
            // Enter in the one-line name field moves on to the prompt.
            self.focus_other(window, cx);
        }
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        cx.emit(NewCommandEvent::Cancel);
    }

    fn focus_other(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.name.focus_handle(cx).is_focused(window) {
            self.prompt
                .update(cx, |prompt, cx| prompt.focus(window, cx));
        } else {
            self.name.update(cx, |name, cx| name.focus(window, cx));
        }
    }

    fn on_tab(&mut self, _: &IndentInline, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_other(window, cx);
    }

    fn on_shift_tab(&mut self, _: &OutdentInline, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_other(window, cx);
    }

    fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        self.scope = scope;
        cx.notify();
    }
}

impl Render for NewCommandForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let label = |text: &'static str| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let scopes = Scope::ALL.iter().enumerate().map(|(index, &scope)| {
            let active = scope == self.scope;
            h_flex()
                .id(("jig-scope", index))
                .px_2()
                .py_0p5()
                .gap_1()
                .rounded_md()
                .text_sm()
                .border_1()
                .border_color(if active { theme.primary } else { theme.border })
                .when(active, |chip| chip.bg(theme.primary.opacity(0.15)))
                .when(!active, |chip| chip.hover(|chip| chip.bg(theme.list_hover)))
                .child(scope.label())
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(format!("⌘{}", index + 1)),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, _, cx| this.set_scope(scope, cx)),
                )
        });
        let name_focused = self.name.focus_handle(cx).is_focused(window);

        let form = v_flex()
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
            .w(px(460.))
            .p_3()
            .gap_2()
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .border_1()
            .border_color(theme.border)
            .rounded(px(10.))
            .shadow_lg()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("New command"),
            )
            .child(label("Name"))
            .child(Input::new(&self.name))
            .child(label("Works on"))
            .child(h_flex().gap_1p5().children(scopes))
            .child(label("Prompt"))
            .child(Textarea::new(&self.prompt).h(px(120.)))
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_xs().text_color(theme.danger).child(error))
            })
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(if name_focused {
                        "tab to the prompt · ⌘↩ save · esc cancel"
                    } else {
                        "⌘↩ save · esc cancel"
                    }),
            );
        crate::motion::pop_in(form, "jig-new-command")
    }
}
