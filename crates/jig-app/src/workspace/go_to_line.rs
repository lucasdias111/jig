//! Go to Line (⌃G): a field at the cursor taking `42` or `42:5`.

use gpui_kit::*;
use jig_editor::EditorHandle;

use super::{GoToLine, Workspace};
use crate::field_box::{FieldBox, FieldBoxEvent};
use crate::lines;

pub(super) struct OpenGoToLine {
    view: Entity<FieldBox>,
    anchor: Point<Pixels>,
    _events: Subscription,
}

impl Workspace {
    pub(super) fn go_to_line(&mut self, _: &GoToLine, window: &mut Window, cx: &mut Context<Self>) {
        if self.home || self.modal_open() || self.go_to_line.is_some() {
            return;
        }
        let view =
            cx.new(|cx| FieldBox::new("", "Line, or line:column", "↩ go · esc cancel", window, cx));
        let events = cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
            FieldBoxEvent::Submit(target) => {
                let target = target.clone();
                this.finish_go_to_line(&target, window, cx);
            }
            FieldBoxEvent::Dismissed => this.close_go_to_line(true, window, cx),
            FieldBoxEvent::Blurred => this.close_go_to_line(false, window, cx),
        });
        self.go_to_line = Some(OpenGoToLine {
            view,
            anchor: self.floating_anchor(cx),
            _events: events,
        });
        cx.notify();
    }

    fn close_go_to_line(&mut self, refocus: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.go_to_line.take().is_some() {
            if refocus {
                self.focus_main(window, cx);
            }
            cx.notify();
        }
    }

    fn finish_go_to_line(&mut self, target: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.close_go_to_line(true, window, cx);
        let Some(offset) = lines::offset_of(&self.editor().text(cx), target) else {
            self.show_note("Type a line number, like 42 or 42:5.".into(), window, cx);
            return;
        };
        self.editor().select(offset..offset, cx);
        self.editor().focus(window, cx);
    }

    /// The field, under the cursor.
    pub(super) fn render_go_to_line(&self) -> Option<AnyElement> {
        let open = self.go_to_line.as_ref()?;
        Some(
            deferred(
                anchored()
                    .position(open.anchor)
                    .snap_to_window_with_margin(px(8.))
                    .child(open.view.clone()),
            )
            .into_any_element(),
        )
    }
}
