//! The brackets at the cursor and the indent guide of its block.

use super::*;

type Ranges = Vec<std::ops::Range<usize>>;

/// The bracket boxes and the active guide's rows and column in the open tab.
fn structure(
    cx: &mut TestAppContext,
    workspace: &Entity<Workspace>,
) -> (Ranges, Option<(std::ops::Range<usize>, usize)>) {
    cx.update(|cx| {
        let this = workspace.read(cx);
        let boxes = this.tab().structure_boxes(cx);
        let guide = this
            .editor()
            .state()
            .read(cx)
            .active_indent_guide()
            .map(|guide| (guide.rows.clone(), guide.column));
        (boxes, guide)
    })
}

#[gpui_kit::test]
fn the_cursor_shows_its_brackets_and_block(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    let text = "fn main() {\n    if x {\n        y(\"(\");\n    }\n}\n";
    std::fs::write(&path, text).unwrap();
    let (window, workspace) = open(cx, &path);

    // Just inside `y(`: its parentheses, past the one in the string, and the
    // guide of the `if` body.
    let call = text.find("y(").unwrap() + 2;
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(call..call, cx);
    });
    step(cx, window, |window, cx| window.render_frame(cx));
    let close = text.find(");").unwrap();
    assert_eq!(
        structure(cx, &workspace),
        (vec![call - 1..call, close..close + 1], Some((2..3, 4)))
    );

    // Typing takes the cursor away from the bracket; back by it, the
    // closing one has moved along.
    type_slowly(cx, window, "a");
    assert_eq!(structure(cx, &workspace).0, Ranges::new());
    press(cx, window, "left");
    let (boxes, guide) = structure(cx, &workspace);
    assert_eq!(boxes, vec![call - 1..call, close + 1..close + 2]);
    assert_eq!(guide, Some((2..3, 4)));

    // Away from any bracket, at the top level: nothing.
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(0..0, cx);
    });
    step(cx, window, |window, cx| window.render_frame(cx));
    assert_eq!(structure(cx, &workspace).0, Ranges::new());
}
