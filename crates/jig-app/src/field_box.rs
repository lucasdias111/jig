//! A one-line field in a small panel at the cursor: Rename Symbol's, which
//! opens holding the name selected as VS Code shows it on F2, and Go to
//! Line's.

use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState};
use gpui_kit::component::v_flex;
use gpui_kit::*;

pub const WIDTH: f32 = 280.;

pub enum FieldBoxEvent {
    /// Enter, with what was typed, trimmed. Unchanged or empty text is
    /// dismissed instead.
    Submit(String),
    Dismissed,
    Blurred,
}

pub struct FieldBox {
    input: Entity<InputState>,
    initial: String,
    hint: &'static str,
    _subscription: Subscription,
}

impl EventEmitter<FieldBoxEvent> for FieldBox {}

impl Focusable for FieldBox {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl FieldBox {
    /// A field holding `initial`, selected, with `hint` under it.
    pub fn new(
        initial: &str,
        placeholder: &'static str,
        hint: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(initial.to_string())
        });
        let subscription = cx.subscribe_in(&input, window, |_, _, event, _, cx| {
            if let InputEvent::Blur = event {
                cx.emit(FieldBoxEvent::Blurred);
            }
        });
        let len = initial.len();
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.set_selected_range(0..len, cx);
        });
        Self {
            input,
            initial: initial.to_string(),
            hint,
            _subscription: subscription,
        }
    }

    fn on_enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let text = self.input.read(cx).value().trim().to_string();
        if text.is_empty() || text == self.initial {
            cx.emit(FieldBoxEvent::Dismissed);
        } else {
            cx.emit(FieldBoxEvent::Submit(text));
        }
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        cx.emit(FieldBoxEvent::Dismissed);
    }
}

impl Render for FieldBox {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = jig_commands::surface::panel(cx)
            .key_context("JigFieldBox")
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .w(px(WIDTH))
            .p_1p5()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .px_1()
                            .text_size(px(13.))
                            .child(Input::new(&self.input).appearance(false).cleanable(false)),
                    )
                    .child(
                        div()
                            .px_1()
                            .child(jig_commands::surface::hint(self.hint, cx)),
                    ),
            );
        jig_commands::motion::pop_in(panel, "jig-field-box")
    }
}
