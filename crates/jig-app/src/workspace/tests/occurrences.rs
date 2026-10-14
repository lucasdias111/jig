//! The other uses of the name at the cursor.

use super::*;

fn occurrences(
    cx: &mut TestAppContext,
    workspace: &Entity<Workspace>,
) -> Vec<std::ops::Range<usize>> {
    cx.update(|cx| workspace.read(cx).tab().occurrences(cx))
}

fn put_cursor(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    range: std::ops::Range<usize>,
) {
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(range, cx);
    });
    step(cx, window, |window, cx| window.render_frame(cx));
}

#[gpui_kit::test]
fn the_name_at_the_cursor_shows_its_uses(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    let text = "let total = 1;\nlet subtotal = total + 2;\nuse(total);\n";
    std::fs::write(&path, text).unwrap();
    let (window, workspace) = open(cx, &path);
    let uses: Vec<_> = text
        .match_indices("total")
        .filter(|(at, _)| !text[..*at].ends_with("sub"))
        .map(|(at, _)| at..at + 5)
        .collect();

    // Not at once: after the cursor has rested a moment.
    put_cursor(cx, window, &workspace, 6..6);
    assert!(occurrences(cx, &workspace).is_empty());
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(occurrences(cx, &workspace), uses);

    // Onto another use, they stay.
    put_cursor(cx, window, &workspace, uses[2].start..uses[2].start);
    assert_eq!(occurrences(cx, &workspace), uses);

    // A selected name shows them too; part of one doesn't.
    put_cursor(cx, window, &workspace, uses[1].clone());
    assert_eq!(occurrences(cx, &workspace), uses);
    put_cursor(cx, window, &workspace, uses[1].start..uses[1].start + 2);
    assert!(occurrences(cx, &workspace).is_empty());

    // A name used once has nothing to show.
    let once = text.find("subtotal").unwrap();
    put_cursor(cx, window, &workspace, once..once);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();
    assert!(occurrences(cx, &workspace).is_empty());

    // Typing clears them.
    put_cursor(cx, window, &workspace, 6..6);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!(occurrences(cx, &workspace).len(), 3);
    type_slowly(cx, window, "x");
    assert!(occurrences(cx, &workspace).is_empty());
}

#[gpui_kit::test]
fn the_language_server_says_where_the_uses_are(cx: &mut TestAppContext) {
    use serde_json::json;

    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let path = dir.path().join("src/main.rs");
    // The server knows the comment's `n` isn't the variable.
    let text = "fn f(n: u8) {\n    // n\n    g(n);\n}\n";
    std::fs::write(&path, text).unwrap();
    let at = |line: u32, character: u32| json!({"line": line, "character": character});
    let (client, _seen) =
        crate::lsp::fake_server(dir.path().to_path_buf(), move |method, _| match method {
            "textDocument/documentHighlight" => json!([
                {"range": {"start": at(0, 5), "end": at(0, 6)}, "kind": 1},
                {"range": {"start": at(2, 6), "end": at(2, 7)}, "kind": 2},
            ]),
            _ => serde_json::Value::Null,
        });
    let server = crate::lsp::server_for("rust").unwrap();
    let root = crate::lsp::root_for(server, &path);
    cx.update(|cx| crate::lsp::register(&server.name, root, client, cx));

    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &path);
    put_cursor(cx, window, &workspace, 5..5);
    let call = text.find("g(n").unwrap() + 2;
    wait_until(cx, |cx| !occurrences(cx, &workspace).is_empty());
    assert_eq!(occurrences(cx, &workspace), vec![5..6, call..call + 1]);
}
