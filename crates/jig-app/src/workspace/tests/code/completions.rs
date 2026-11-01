//! Completions with and without a server, and snippets.

use super::*;

/// The labels in the completion list, if it's showing.
pub(super) fn completion_labels(
    cx: &mut TestAppContext,
    workspace: &Entity<Workspace>,
) -> Vec<String> {
    cx.update(|cx| {
        let state = workspace.read(cx).editor().state().read(cx);
        let menu = state.completion_menu_state();
        if !menu.open {
            return Vec::new();
        }
        menu.items.iter().map(|item| item.label.clone()).collect()
    })
}

#[gpui_kit::test]
fn words_in_the_file_complete_without_a_language_server(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "counter = 1\n").unwrap();
    let (window, workspace) = open(cx, &path);
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(12..12, cx);
    });

    type_slowly(cx, window, "c");
    assert_eq!(completion_labels(cx, &workspace), ["counter"]);
    type_slowly(cx, window, "ou");
    assert_eq!(completion_labels(cx, &workspace), ["counter"]);

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    });
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        assert_eq!(editor.text(cx), "counter = 1\ncounter");
    });
    assert!(completion_labels(cx, &workspace).is_empty());
}

#[gpui_kit::test]
fn a_snippet_is_filled_in_place_by_place(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    std::fs::write(&path, "fn main() {\n    \n}\n").unwrap();
    let (window, workspace) = open(cx, &path);
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(16..16, cx);
    });

    type_slowly(cx, window, "fo");
    assert_eq!(completion_labels(cx, &workspace)[..2], ["for", "fori"]);
    press(cx, window, "enter");
    let (text, selected) = text_and_selection(cx, &workspace);
    assert_eq!(
        text,
        "fn main() {\n    for item in items {\n        \n    }\n}\n"
    );
    assert_eq!(selected, "item");

    type_slowly(cx, window, "x");
    press(cx, window, "tab");
    assert_eq!(text_and_selection(cx, &workspace).1, "items");
    press(cx, window, "shift-tab");
    assert_eq!(text_and_selection(cx, &workspace).1, "x");
    press(cx, window, "tab");
    type_slowly(cx, window, "xs");
    // The last Tab lands in the body and ends the snippet.
    press(cx, window, "tab");
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert_eq!(
            this.editor().text(cx),
            "fn main() {\n    for x in xs {\n        \n    }\n}\n"
        );
        assert_eq!(this.editor().selection(cx), 38..38);
        assert!(this.tab().snippet.is_none());
    });
    // Tab indents again.
    press(cx, window, "tab");
    assert_eq!(
        text_and_selection(cx, &workspace).0,
        "fn main() {\n    for x in xs {\n            \n    }\n}\n"
    );
}

#[gpui_kit::test]
fn var_after_an_expression_names_it(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.ts");
    std::fs::write(&path, "getUser(id)\n").unwrap();
    let (window, workspace) = open(cx, &path);
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(11..11, cx);
    });

    type_slowly(cx, window, ".va");
    assert_eq!(completion_labels(cx, &workspace), ["var"]);
    press(cx, window, "enter");
    let (text, selected) = text_and_selection(cx, &workspace);
    assert_eq!(text, "const user = getUser(id);\n");
    assert_eq!(selected, "user");
    // Escape leaves the snippet; Tab then indents.
    press(cx, window, "escape");
    cx.update(|cx| assert!(workspace.read(cx).tab().snippet.is_none()));
}

/// A tab whose Rust server answers completions with `answer`, given the
/// line asked about.
fn with_completing_server(
    cx: &mut TestAppContext,
    text: &str,
    answer: impl Fn(u64) -> serde_json::Value + Send + 'static,
) -> (AnyWindowHandle, Entity<Workspace>) {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let path = dir.path().join("src/main.rs");
    std::fs::write(&path, text).unwrap();
    let (client, _seen) = jig_lsp::lsp::fake_server(
        dir.path().to_path_buf(),
        move |method, params| match method {
            "textDocument/completion" => answer(params["position"]["line"].as_u64().unwrap()),
            _ => serde_json::Value::Null,
        },
    );
    let server = jig_lsp::lsp::server_for("rust").unwrap();
    let root = jig_lsp::lsp::root_for(server, &path);
    cx.update(|cx| jig_lsp::lsp::register(&server.name, root, client, cx));
    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &path);
    // The folder goes with the test's end; the tab has read the file.
    std::mem::forget(dir);
    (window, workspace)
}

fn put_cursor(cx: &mut TestAppContext, workspace: &Entity<Workspace>, at: usize) {
    cx.update(|cx| workspace.read(cx).editor().clone().select(at..at, cx));
}

/// Wait for the server's answer to show, or not.
fn settle(cx: &mut TestAppContext) {
    for _ in 0..50 {
        cx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

#[gpui_kit::test]
fn completions_show_before_where_a_list_last_opened(cx: &mut TestAppContext) {
    let text = "fn a() {\n    \n}\nfn b() {\n    \n}\n";
    let (window, workspace) = with_completing_server(
        cx,
        text,
        |_| serde_json::json!([{"label": "value", "kind": 6}]),
    );
    put_cursor(cx, &workspace, text.rfind("    \n").unwrap() + 4);
    type_slowly(cx, window, "v");
    settle(cx);
    assert_eq!(completion_labels(cx, &workspace), ["value"]);
    press(cx, window, "escape");
    put_cursor(cx, &workspace, text.find("    \n").unwrap() + 4);
    type_slowly(cx, window, "v");
    settle(cx);
    assert_eq!(completion_labels(cx, &workspace), ["value"]);
}

#[gpui_kit::test]
fn a_struct_literal_lists_its_fields(cx: &mut TestAppContext) {
    use serde_json::json;
    let text = "fn a() {\n    if ready \n    let p = Point \n}\n";
    // The `if` gets everything a block could hold; below it, the literal
    // gets its fields.
    let (window, workspace) = with_completing_server(cx, text, |line| match line {
        1 => json!([{"label": "x", "kind": 5}, {"label": "main", "kind": 3}]),
        _ => json!([
            {"label": "x", "kind": 5, "sortText": "1"},
            {"label": "height", "kind": 5, "sortText": "2"},
        ]),
    });
    put_cursor(cx, &workspace, text.find("ready ").unwrap() + 6);
    type_slowly(cx, window, "{ ");
    settle(cx);
    assert!(completion_labels(cx, &workspace).is_empty(), "a block");

    let literal = cx
        .update(|cx| workspace.read(cx).editor().text(cx))
        .find("Point ")
        .unwrap()
        + 6;
    put_cursor(cx, &workspace, literal);
    type_slowly(cx, window, "{");
    settle(cx);
    assert!(
        completion_labels(cx, &workspace).is_empty(),
        "Enter is free"
    );
    type_slowly(cx, window, " ");
    settle(cx);
    assert_eq!(completion_labels(cx, &workspace), ["x", "height"]);
    // And on a new line after a comma.
    press(cx, window, "escape");
    type_slowly(cx, window, "x: 1,");
    settle(cx);
    assert!(
        completion_labels(cx, &workspace).is_empty(),
        "Enter is free"
    );
    press(cx, window, "enter");
    settle(cx);
    assert_eq!(completion_labels(cx, &workspace), ["x", "height"]);
}

#[gpui_kit::test]
fn show_completions_lists_what_fits_at_the_cursor(cx: &mut TestAppContext) {
    let text = "fn a() {\n    \n}\n";
    let (window, workspace) = with_completing_server(cx, text, |_| {
        serde_json::json!([
            {"label": "value", "kind": 6, "sortText": "1"},
            {"label": "main", "kind": 3, "sortText": "2"},
        ])
    });
    put_cursor(cx, &workspace, text.find("    \n").unwrap() + 4);
    settle(cx);
    assert!(completion_labels(cx, &workspace).is_empty());
    press(cx, window, "ctrl-space");
    settle(cx);
    assert_eq!(completion_labels(cx, &workspace), ["value", "main"]);
    press(cx, window, "enter");
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        assert_eq!(editor.text(cx), "fn a() {\n    value\n}\n");
    });
}
