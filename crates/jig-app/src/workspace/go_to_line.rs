//! Go to Line (⌃G): a field at the cursor taking `42` or `42:5`.

use gpui_kit::*;
use jig_editor::EditorHandle;

use super::{GoToLine, Workspace};
use jig_kit::field_box::{FieldBox, FieldBoxEvent};
use jig_kit::lines;

/// The field, while it's open. Its own entity, so the workspace only opens
/// it, places it and acts on what it says.
#[derive(Default)]
pub(super) struct GoToLineField {
    open: Option<OpenField>,
}

struct OpenField {
    view: Entity<FieldBox>,
    anchor: Point<Pixels>,
    _events: Subscription,
}

/// What the field asks of the workspace, which has the code and the focus.
pub(super) enum GoToLineEvent {
    /// Go to what was typed, `42` or `42:5`. The field has closed.
    Go(String),
    /// The field closed without going anywhere. `refocus` when it was
    /// dismissed, rather than left by clicking elsewhere.
    Closed { refocus: bool },
}

impl EventEmitter<GoToLineEvent> for GoToLineField {}

impl GoToLineField {
    pub(super) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Open the field at `anchor`, with the keyboard.
    fn open(&mut self, anchor: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let view =
            cx.new(|cx| FieldBox::new("", "Line, or line:column", "↩ go · esc cancel", window, cx));
        let events = cx.subscribe(&view, |this, _, event, cx| match event {
            FieldBoxEvent::Submit(target) => {
                if this.open.take().is_some() {
                    cx.emit(GoToLineEvent::Go(target.clone()));
                    cx.notify();
                }
            }
            FieldBoxEvent::Dismissed => this.close(true, cx),
            FieldBoxEvent::Blurred => this.close(false, cx),
        });
        self.open = Some(OpenField {
            view,
            anchor,
            _events: events,
        });
        cx.notify();
    }

    fn close(&mut self, refocus: bool, cx: &mut Context<Self>) {
        if self.open.take().is_some() {
            cx.emit(GoToLineEvent::Closed { refocus });
            cx.notify();
        }
    }
}

impl Render for GoToLineField {
    /// The field, under the cursor.
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let Some(open) = &self.open else {
            return div().into_any_element();
        };
        deferred(
            anchored()
                .position(open.anchor)
                .snap_to_window_with_margin(px(8.))
                .child(open.view.clone()),
        )
        .into_any_element()
    }
}

impl Workspace {
    pub(super) fn go_to_line(&mut self, _: &GoToLine, window: &mut Window, cx: &mut Context<Self>) {
        if self.home || self.modal_open() || self.go_to_line.read(cx).is_open() {
            return;
        }
        let anchor = self.floating_anchor(cx);
        self.go_to_line
            .update(cx, |field, cx| field.open(anchor, window, cx));
        cx.notify();
    }

    /// Act on what the field says: back to the code, and on to the line.
    pub(super) fn on_go_to_line_event(
        &mut self,
        _: &Entity<GoToLineField>,
        event: &GoToLineEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            GoToLineEvent::Go(target) => {
                self.focus_main(window, cx);
                cx.notify();
                self.finish_go_to_line(target, window, cx);
            }
            GoToLineEvent::Closed { refocus } => {
                if *refocus {
                    self.focus_main(window, cx);
                }
                cx.notify();
            }
        }
    }

    fn finish_go_to_line(&mut self, target: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(offset) = lines::offset_of(&self.editor().text(cx), target) else {
            self.show_note("Type a line number, like 42 or 42:5.".into(), window, cx);
            return;
        };
        self.remember_location(cx);
        self.editor().select(offset..offset, cx);
        self.focus_editor(window, cx);
    }

    /// The field, under the cursor, while it's open.
    pub(super) fn render_go_to_line(&self, cx: &App) -> Option<AnyElement> {
        self.go_to_line
            .read(cx)
            .is_open()
            .then(|| self.go_to_line.clone().into_any_element())
    }
}
