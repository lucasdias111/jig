//! Checks the two things Milestone 0 must prove about the editor component:
//! the anchor point is readable and tracks the selection, and an applied
//! edit is exactly one undo step.

use gpui_kit::component::input::{Editor, EditorState};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Point, Render, Styled,
    TestAppContext, Window, WindowBounds, WindowOptions, div, px, size,
};
use jig_editor::{EditorHandle, KitEditor};

const SOURCE: &str = "fn main() {\n    let x = 1;\n    println!(\"{x}\");\n}\n";

struct Fixture {
    editor: KitEditor,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(Editor::new(self.editor.state()).size_full())
    }
}

fn open(cx: &mut TestAppContext) -> (gpui_kit::AnyWindowHandle, KitEditor) {
    cx.update(gpui_kit::init);
    let (window, view) = cx.update(|cx| {
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds {
                origin: Point::default(),
                size: size(px(800.), px(480.)),
            })),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            let state: Entity<EditorState> =
                cx.new(|cx| EditorState::new(window, cx).language("rust").default_value(SOURCE));
            let editor = KitEditor::new(state, cx);
            cx.new(|_| Fixture { editor })
        })
        .expect("open window")
    });
    let editor = cx.update(|cx| view.read(cx).editor.clone());
    cx.run_until_parked();
    (window, editor)
}

#[gpui_kit::test]
fn anchor_point_follows_selection(cx: &mut TestAppContext) {
    let (window, editor) = open(cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        let line_1 = SOURCE.find("let").unwrap();
        let line_2 = SOURCE.find("println").unwrap();

        editor.state().update(cx, |s, cx| s.set_selected_range(line_1..line_1, cx));
        window.render_frame(cx);
        let a = editor.anchor_point(cx).expect("anchor for cursor on line 1");

        editor.state().update(cx, |s, cx| s.set_selected_range(line_2..line_2, cx));
        window.render_frame(cx);
        let b = editor.anchor_point(cx).expect("anchor for cursor on line 2");

        assert!(b.y > a.y, "anchor moves down with the cursor: {a:?} -> {b:?}");
        assert_eq!(a.x, b.x, "same column, same x");

        // A multi-line selection anchors below its last line.
        editor.state().update(cx, |s, cx| s.set_selected_range(line_1..line_2 + 3, cx));
        window.render_frame(cx);
        let c = editor.anchor_point(cx).expect("anchor for selection");
        assert_eq!(c.y, b.y);
    })
    .unwrap();
}

#[gpui_kit::test]
fn apply_edit_is_one_undo_step(cx: &mut TestAppContext) {
    let (window, editor) = open(cx);
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        editor.focus(window, cx);
        let start = SOURCE.find("let").unwrap();
        let end = start + "let x = 1;".len();

        let new_text = "/// The answer.\n    let x = 42;";
        let range = editor.apply_edit(start..end, new_text, window, cx);
        assert_eq!(&editor.text(cx)[range], new_text);
        assert!(editor.text(cx).contains("let x = 42;"));

        window.press("secondary-z", cx);
        assert_eq!(editor.text(cx), SOURCE, "a single undo restores the original");
    })
    .unwrap();
}

#[gpui_kit::test]
fn edits_apply_while_readonly(cx: &mut TestAppContext) {
    let (window, editor) = open(cx);
    cx.update_window(window, |_, window, cx| {
        editor.set_readonly(true, cx);
        let range = editor.apply_edit(0..2, "pub fn", window, cx);
        assert_eq!(range, 0..6);
        assert!(editor.text(cx).starts_with("pub fn main"));
    })
    .unwrap();
}
