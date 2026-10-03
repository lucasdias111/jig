//! Jig: a code editor where AI works through small commands at the cursor.
//!
//! M0 spike: one window, one editor, plus debug aids that check the editor
//! adapter (anchor position follows the cursor; an edit is one undo step).

use std::path::PathBuf;

use anyhow::Context as _;
use gpui_kit::component::input::{Editor, EditorState, InputEvent};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_editor::{EditorHandle, KitEditor};

actions!(jig, [Quit, DebugEdit, ToggleDebugOverlay]);

struct Workspace {
    editor: KitEditor,
    show_overlay: bool,
    _subscription: Subscription,
}

impl Workspace {
    fn new(text: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.new(|cx| EditorState::new(window, cx).language("rust").default_value(text));
        // Re-render on every edit or cursor move so the debug overlay tracks the cursor.
        let subscription = cx.subscribe(&state, |_, _, _: &InputEvent, cx| cx.notify());
        let editor = KitEditor::new(state, cx);
        editor.focus(window, cx);
        Self { editor, show_overlay: true, _subscription: subscription }
    }

    fn debug_edit(&mut self, _: &DebugEdit, window: &mut Window, cx: &mut Context<Self>) {
        let selection = self.editor.selection(cx);
        let original = self.editor.text(cx)[selection.clone()].to_string();
        let new_text = format!("/* jig */{original}");
        let range = self.editor.apply_edit(selection, &new_text, window, cx);
        self.editor.highlight(vec![(range, hsla(0.38, 0.6, 0.5, 0.18))], cx);
        cx.notify();
    }

    fn toggle_overlay(&mut self, _: &ToggleDebugOverlay, _: &mut Window, cx: &mut Context<Self>) {
        self.show_overlay = !self.show_overlay;
        cx.notify();
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let anchor = self.editor.anchor_point(cx).filter(|_| self.show_overlay);
        div()
            .size_full()
            .on_action(cx.listener(Self::debug_edit))
            .on_action(cx.listener(Self::toggle_overlay))
            .child(Editor::new(self.editor.state()).bordered(false).size_full())
            .when_some(anchor, |this, anchor| {
                // A marker where the floating command input will open.
                this.child(deferred(
                    anchored().position(anchor).child(
                        div()
                            .w(px(160.))
                            .h(px(4.))
                            .rounded_full()
                            .bg(hsla(0.6, 0.9, 0.55, 0.9)),
                    ),
                ))
            })
    }
}

fn main() -> anyhow::Result<()> {
    let path = std::env::args().nth(1).map(PathBuf::from);
    let text = match &path {
        Some(path) => std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?,
        None => include_str!("main.rs").to_string(),
    };

    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-shift-e", DebugEdit, None),
            KeyBinding::new("cmd-shift-o", ToggleDebugOverlay, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(960.), px(720.)), cx)),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| Workspace::new(text, window, cx)))
            .expect("failed to open window");
        cx.activate(true);
    });
    Ok(())
}
