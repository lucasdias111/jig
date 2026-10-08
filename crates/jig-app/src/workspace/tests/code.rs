//! Go to definition, completion, snippets and Rename Symbol.

use super::*;

#[gpui_kit::test]
fn f12_goes_to_a_definition_in_another_file_then_to_usages(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() {\n    alpha();\n}\n",
    )
    .unwrap();
    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &dir.path().join("src/main.rs"));

    // On the call: to the function in lib.rs.
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(18..18, cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("f12", cx);
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(
            this.document()
                .path
                .as_ref()
                .unwrap()
                .ends_with("src/lib.rs")
        );
        assert_eq!(this.editor().selection(cx), 7..12);
    });

    // On the declaration: its usages.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("f12", cx);
    });
    let query = cx.update(|cx| {
        let this = workspace.read(cx);
        this.find_in_files
            .as_ref()
            .map(|open| open.view.read(cx).query(cx))
    });
    assert_eq!(query.as_deref(), Some("alpha"));
}

/// The labels in the completion list, if it's showing.
fn completion_labels(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
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

/// The current tab's text, and what's selected in it.
fn text_and_selection(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> (String, String) {
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        let text = editor.text(cx);
        let selected = text[editor.selection(cx)].to_string();
        (text, selected)
    })
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

#[gpui_kit::test]
fn the_language_server_completes_after_a_dot(cx: &mut TestAppContext) {
    use serde_json::json;

    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let path = dir.path().join("src/main.rs");
    std::fs::write(&path, "fn main() {\n    v\n}\n").unwrap();
    let (client, seen) =
        crate::lsp::fake_server(dir.path().to_path_buf(), move |method, _| match method {
            "textDocument/completion" => json!({"isIncomplete": false, "items": [
                {"label": "pop", "kind": 2},
                {"label": "push", "kind": 2, "detail": "fn(&mut self, T)"},
            ]}),
            _ => serde_json::Value::Null,
        });
    let server = crate::lsp::server_for("rust").unwrap();
    let root = crate::lsp::root_for(server, &path);
    cx.update(|cx| crate::lsp::register(server.name, root, client, cx));

    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &path);
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(17..17, cx);
    });

    let wait_for = |cx: &mut TestAppContext, done: &dyn Fn(&mut TestAppContext) -> bool| {
        for _ in 0..200 {
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    type_slowly(cx, window, ".");
    wait_for(cx, &|cx| completion_labels(cx, &workspace).len() > 2);
    // The server's, then Jig's postfix snippets.
    let labels = completion_labels(cx, &workspace);
    assert_eq!(labels[..3], ["pop", "push", "for"]);
    assert!(labels.contains(&"var".to_string()));
    type_slowly(cx, window, "pu");
    wait_for(cx, &|cx| completion_labels(cx, &workspace).len() == 1);
    assert_eq!(completion_labels(cx, &workspace), ["push"]);

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    });
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        assert_eq!(editor.text(cx), "fn main() {\n    v.push\n}\n");
    });

    // Each request saw the text it was made for.
    let messages: Vec<serde_json::Value> = seen.try_iter().collect();
    let mut text = String::new();
    for message in &messages {
        match message["method"].as_str() {
            Some("textDocument/didChange") => {
                text = message["params"]["contentChanges"][0]["text"]
                    .as_str()
                    .unwrap()
                    .to_string();
            }
            Some("textDocument/completion") => {
                let offset = crate::lsp::offset(
                    &text,
                    serde_json::from_value(message["params"]["position"].clone()).unwrap(),
                    crate::lsp::Encoding::Utf8,
                );
                assert!(
                    text[..offset].ends_with(['.', 'p', 'u']),
                    "{text:?} at {offset}"
                );
            }
            _ => {}
        }
    }
    assert!(
        messages
            .iter()
            .any(|m| m["params"]["context"] == json!({"triggerKind": 2, "triggerCharacter": "."}))
    );
}

#[gpui_kit::test]
fn the_language_server_answers_definitions_and_references(cx: &mut TestAppContext) {
    use serde_json::json;

    // The fake server runs on real threads.
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    // Not where a guess would go: only the server knows.
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() {\n    renamed();\n}\n",
    )
    .unwrap();
    let lib = crate::lsp::file_uri(&dir.path().join("src/lib.rs"));
    let main = crate::lsp::file_uri(&dir.path().join("src/main.rs"));
    let at = |line: u32, start: u32, end: u32| json!({"start": {"line": line, "character": start}, "end": {"line": line, "character": end}});
    let (client, _seen) =
        crate::lsp::fake_server(dir.path().to_path_buf(), move |method, _| match method {
            "textDocument/definition" => json!({"uri": lib, "range": at(0, 7, 12)}),
            "textDocument/references" => json!([
                {"uri": lib, "range": at(0, 7, 12)},
                {"uri": main, "range": at(1, 4, 11)},
            ]),
            _ => serde_json::Value::Null,
        });
    let main_path = dir.path().join("src/main.rs");
    let server = crate::lsp::server_for("rust").unwrap();
    let root = crate::lsp::root_for(server, &main_path);
    cx.update(|cx| crate::lsp::register(server.name, root, client, cx));

    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &main_path);
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(18..18, cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("f12", cx);
    });
    // The server answers on its own thread.
    let wait_for = |cx: &mut TestAppContext, done: &dyn Fn(&gpui_kit::App) -> bool| {
        for _ in 0..200 {
            cx.run_until_parked();
            if cx.update(|cx| done(cx)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    };
    wait_for(cx, &|cx| {
        workspace
            .read(cx)
            .document()
            .path
            .as_ref()
            .unwrap()
            .ends_with("src/lib.rs")
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(
            this.document()
                .path
                .as_ref()
                .unwrap()
                .ends_with("src/lib.rs")
        );
        assert_eq!(this.editor().selection(cx), 7..12);
    });

    // The server says this is the declaration: its references.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("f12", cx);
    });
    wait_for(cx, &|cx| workspace.read(cx).find_in_files.is_some());
    assert_eq!(
        find_results(cx, &workspace),
        [
            "src/lib.rs:1: pub fn alpha() {}",
            "src/main.rs:2: renamed();"
        ]
    );
}

#[gpui_kit::test]
fn cmd_click_goes_to_the_definition(cx: &mut TestAppContext) {
    cmd_click_to_definition(cx, 0);
}

#[gpui_kit::test]
fn cmd_click_works_after_scrolling(cx: &mut TestAppContext) {
    // Far enough down that the editor must scroll more than a screen.
    cmd_click_to_definition(cx, 200);
}

/// Cmd+hover then Cmd+click a call `padding` lines down main.rs; it
/// should land on the function in lib.rs.
fn cmd_click_to_definition(cx: &mut TestAppContext, padding: usize) {
    use serde_json::json;

    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    let main = format!(
        "fn main() {{\n{}    renamed();\n}}\n",
        "    // filler\n".repeat(padding)
    );
    let call = main.find("renamed").unwrap();
    std::fs::write(dir.path().join("src/main.rs"), &main).unwrap();
    let lib = crate::lsp::file_uri(&dir.path().join("src/lib.rs"));
    let (client, _seen) =
        crate::lsp::fake_server(dir.path().to_path_buf(), move |method, _| match method {
            "textDocument/definition" => json!({"uri": lib, "range": {
                    "start": {"line": 0, "character": 7}, "end": {"line": 0, "character": 12}}}),
            _ => serde_json::Value::Null,
        });
    let main_path = dir.path().join("src/main.rs");
    let server = crate::lsp::server_for("rust").unwrap();
    let root = crate::lsp::root_for(server, &main_path);
    cx.update(|cx| crate::lsp::register(server.name, root, client, cx));

    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &main_path);
    // Scroll the call into view.
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(call..call, cx);
    });
    step(cx, window, |window, cx| window.render_frame(cx));
    step(cx, window, |window, cx| window.render_frame(cx));
    let point = cx.update(|cx| {
        let state = workspace.read(cx).editor().state().clone();
        let bounds = state
            .read(cx)
            .range_to_bounds(&(call + 1..call + 2))
            .unwrap();
        bounds.center()
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.simulate_mouse_move(point, None, gpui_kit::Modifiers::secondary_key());
    for _ in 0..100 {
        vcx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    vcx.update(|window, cx| window.render_frame(cx));
    vcx.simulate_click(point, gpui_kit::Modifiers::secondary_key());
    for _ in 0..50 {
        vcx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(this.find_in_files.is_none(), "searched instead of jumping");
        assert!(
            this.document()
                .path
                .as_ref()
                .unwrap()
                .ends_with("src/lib.rs"),
            "Cmd+click at {point:?} didn't jump"
        );
        assert_eq!(this.editor().selection(cx), 7..12);
    });
}

#[gpui_kit::test]
fn rename_symbol_without_a_server_renames_in_the_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.rs");
    let source = "fn main() {\n    let total = 1;\n    use_it(total, subtotal);\n}\n";
    std::fs::write(&path, source).unwrap();
    let (window, workspace) = open(cx, &path);
    let at = source.find("total").unwrap() + 2;
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(at..at, cx);
    });
    step(cx, window, |window, cx| window.press("f2", cx));
    assert!(workspace.read_with(cx, |this, _| this.rename.is_some()));
    // The old name is selected, so typing replaces it.
    cx.simulate_input(window, "sum");
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |this, _| this.rename.is_none()));
    assert_eq!(
        text(cx, &workspace),
        "fn main() {\n    let sum = 1;\n    use_it(sum, subtotal);\n}\n"
    );
    // In the buffer only, until it's saved.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
    assert!(workspace.read_with(cx, |this, _| this.tab().dirty));

    // Esc leaves it be.
    step(cx, window, |window, cx| window.press("f2", cx));
    cx.simulate_input(window, "other");
    cx.simulate_keystrokes(window, "escape");
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |this, _| this.rename.is_none()));
    assert!(text(cx, &workspace).contains("let sum = 1;"));
}

#[gpui_kit::test]
fn rename_symbol_with_a_server_edits_every_file(cx: &mut TestAppContext) {
    use serde_json::json;

    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let lib_source = "pub fn alpha() {}\n";
    std::fs::write(dir.path().join("src/lib.rs"), lib_source).unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() {\n    alpha();\n}\n",
    )
    .unwrap();
    let lib = crate::lsp::file_uri(&dir.path().join("src/lib.rs"));
    let main = crate::lsp::file_uri(&dir.path().join("src/main.rs"));
    let at = |line: u32, start: u32, end: u32| json!({"start": {"line": line, "character": start}, "end": {"line": line, "character": end}});
    let (client, _seen) = crate::lsp::fake_server(
        dir.path().to_path_buf(),
        move |method, params| match method {
            "textDocument/rename" => {
                let name = params["newName"].clone();
                json!({"changes": {
                    lib.clone(): [{"range": at(0, 7, 12), "newText": name}],
                    main.clone(): [{"range": at(1, 4, 9), "newText": name}],
                }})
            }
            _ => serde_json::Value::Null,
        },
    );
    let main_path = dir.path().join("src/main.rs");
    let server = crate::lsp::server_for("rust").unwrap();
    let root = crate::lsp::root_for(server, &main_path);
    cx.update(|cx| crate::lsp::register(server.name, root, client, cx));

    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &main_path);
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(18..18, cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("f2", cx);
    });
    cx.simulate_input(window, "beta");
    cx.simulate_keystrokes(window, "enter");
    for _ in 0..200 {
        cx.run_until_parked();
        if text(cx, &workspace).contains("beta") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(text(cx, &workspace), "fn main() {\n    beta();\n}\n");
    cx.update(|cx| {
        let this = workspace.read(cx);
        // Still on main.rs, with lib.rs changed in a tab behind it.
        assert!(
            this.document()
                .path
                .as_ref()
                .unwrap()
                .ends_with("src/main.rs")
        );
        let lib = this
            .tab_for(&dir.path().join("src/lib.rs"))
            .expect("lib.rs opens");
        assert_eq!(this.tabs[lib].editor.text(cx), "pub fn beta() {}\n");
        assert!(this.tabs[lib].dirty);
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
        lib_source
    );
}
