//! Problems, hover and signature help.

use super::*;

/// What the editor holds as the current file's problems: each one's
/// text and message.
fn problems(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<(String, String)> {
    cx.update(|cx| {
        let state = workspace.read(cx).editor().state().read(cx);
        let text = state.value().to_string();
        state
            .diagnostics()
            .unwrap()
            .iter()
            .map(|e| (text[e.range.clone()].to_string(), e.message.to_string()))
            .collect()
    })
}

fn report(cx: &mut TestAppContext, path: &Path, items: serde_json::Value) {
    let published = crate::lsp::diagnostics::Published {
        key: crate::lsp::diagnostics::path_key(path),
        version: None,
        items: serde_json::from_value(items).unwrap(),
        encoding: crate::lsp::Encoding::Utf16,
    };
    cx.update(|cx| crate::lsp::diagnostics::publish(published, cx));
    cx.run_until_parked();
}

#[gpui_kit::test]
fn problems_show_count_and_can_be_stepped_through(cx: &mut TestAppContext) {
    use serde_json::json;

    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let path = dir.path().join("src/main.rs");
    let text = "fn main() {\n    let x: u32 = \"é\";\n    let y = 1;\n}\n";
    std::fs::write(&path, text).unwrap();
    let other = dir.path().join("src/other.rs");
    std::fs::write(&other, "pub fn f() {}\n").unwrap();
    let (client, _seen) =
        crate::lsp::fake_server(dir.path().to_path_buf(), |_, _| serde_json::Value::Null);
    let server = crate::lsp::server_for("rust").unwrap();
    let root = crate::lsp::root_for(server, &path);
    cx.update(|cx| crate::lsp::register(server.name, root, client, cx));
    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &path);
    let counts = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let counts = workspace.read(cx).problem_counts(cx);
            (counts.errors, counts.warnings)
        })
    };
    let hover = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let hover = workspace.read(cx).tab().problems.hover.clone();
            hover.update(cx, |hover, cx| hover.message(cx))
        })
    };
    let cursor = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).editor().cursor(cx));
    assert_eq!(counts(cx), (0, 0));

    // In UTF-16 columns, which `é` doesn't change but bytes would.
    report(
        cx,
        &path,
        json!([
            {
                "range": {"start": {"line": 2, "character": 8}, "end": {"line": 2, "character": 9}},
                "severity": 2,
                "message": "unused variable `y`",
            },
            {
                "range": {"start": {"line": 1, "character": 17}, "end": {"line": 1, "character": 20}},
                "severity": 1,
                "source": "rustc",
                "message": "mismatched types",
            },
        ]),
    );
    assert_eq!(counts(cx), (1, 1));
    assert_eq!(
        problems(cx, &workspace),
        [
            ("\"é\"".to_string(), "mismatched types".to_string()),
            ("y".to_string(), "unused variable `y`".to_string()),
        ]
    );
    // Clicking the counts in the status bar goes to the first, and
    // shows its message.
    let error = text.find('"').unwrap();
    let warning = text.find("y =").unwrap();
    {
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| window.render_frame(cx));
        let counts = vcx
            .debug_bounds("problem-counts")
            .expect("the counts are in the status bar");
        vcx.simulate_click(counts.center(), gpui_kit::Modifiers::none());
        vcx.run_until_parked();
        vcx.update(|window, cx| window.render_frame(cx));
        assert!(vcx.debug_bounds("problem").is_some(), "the message shows");
    }
    assert_eq!(cursor(cx), error);

    // ⌥F8 goes from problem to problem, round the end; ⇧⌥F8 back.
    assert_eq!(hover(cx).as_deref(), Some("mismatched types"));
    press(cx, window, "alt-f8");
    assert_eq!(cursor(cx), warning);
    press(cx, window, "alt-f8");
    assert_eq!(cursor(cx), error);
    press(cx, window, "alt-shift-f8");
    assert_eq!(cursor(cx), warning);
    assert_eq!(hover(cx).as_deref(), Some("unused variable `y`"));
    press(cx, window, "escape");
    assert_eq!(hover(cx), None);

    // They move with edits until the server reports again.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-up", cx);
    });
    type_slowly(cx, window, "//\n");
    assert_eq!(
        problems(cx, &workspace),
        [
            ("\"é\"".to_string(), "mismatched types".to_string()),
            ("y".to_string(), "unused variable `y`".to_string()),
        ]
    );
    assert_eq!(counts(cx), (1, 1));
    report(cx, &path, json!([]));
    assert_eq!(counts(cx), (0, 0));
    assert!(problems(cx, &workspace).is_empty());

    // A file's problems wait for it to open.
    report(
        cx,
        &other,
        json!([{
            "range": {"start": {"line": 0, "character": 7}, "end": {"line": 0, "character": 8}},
            "severity": 1,
            "message": "expected `;`",
        }]),
    );
    open_file(cx, window, &workspace, &other);
    assert_eq!(counts(cx), (1, 0));
    assert_eq!(
        problems(cx, &workspace),
        [("f".to_string(), "expected `;`".to_string())]
    );
}

#[gpui_kit::test]
fn fix_sends_the_problems_on_the_line(cx: &mut TestAppContext) {
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    let text = "fn main() {\n    let x: u32 = \"s\";\n}\n";
    std::fs::write(&path, text).unwrap();
    let (window, workspace) = open(cx, &path);
    let provider = Arc::new(RecordingProvider(Default::default()));
    let provider_for_ws: Arc<dyn Provider> = provider.clone();
    cx.update(|cx| workspace.update(cx, |this, _| this.provider = Ok(provider_for_ws)));
    report(
        cx,
        &path,
        json!([{
            "range": {"start": {"line": 1, "character": 17}, "end": {"line": 1, "character": 20}},
            "severity": 1,
            "source": "rustc",
            "code": "E0308",
            "message": "mismatched types",
        }]),
    );

    // The cursor on the line, nothing selected.
    let on_line = text.find("let").unwrap();
    run_preset(cx, window, &workspace, on_line..on_line, "fix");
    // Another jig doesn't send them.
    press(cx, window, "escape");
    run_preset(cx, window, &workspace, on_line..on_line, "simplify");

    let sent = provider.0.lock().unwrap().clone();
    assert_eq!(sent.len(), 2);
    assert!(
        sent[0].contains(
            "<diagnostics>\nline 2, error: mismatched types (rustc E0308)\n</diagnostics>\n"
        ),
        "{}",
        sent[0]
    );
    assert!(!sent[1].contains("<diagnostics>"), "{}", sent[1]);
}

/// A Rust file at `text` in a project whose server is `answer`.
fn with_fake_server(
    cx: &mut TestAppContext,
    text: &str,
    answer: impl Fn(&str, &serde_json::Value) -> serde_json::Value + Send + 'static,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    std::path::PathBuf,
    tempfile::TempDir,
) {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let path = dir.path().join("src/main.rs");
    std::fs::write(&path, text).unwrap();
    let (client, _seen) = crate::lsp::fake_server(dir.path().to_path_buf(), answer);
    let server = crate::lsp::server_for("rust").unwrap();
    let root = crate::lsp::root_for(server, &path);
    cx.update(|cx| crate::lsp::register(server.name, root, client, cx));
    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &path);
    (window, workspace, path, dir)
}

type HoverShown = Option<(Option<String>, Vec<String>)>;

fn hover_shown(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> HoverShown {
    cx.update(|cx| {
        let hover = workspace.read(cx).tab().hover.clone();
        hover.read(cx).hover_text(cx)
    })
}

#[gpui_kit::test]
fn hover_shows_what_the_server_says_with_the_problem_there(cx: &mut TestAppContext) {
    use serde_json::json;

    let text = "fn main() {\n    helper();\n}\n";
    let (window, workspace, path, _dir) = with_fake_server(cx, text, |method, _| match method {
        "textDocument/hover" => json!({
            "contents": {"kind": "markdown", "value": "```rust\nfn helper()\n```\n\nHelps."},
            "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}},
        }),
        _ => serde_json::Value::Null,
    });
    let name = text.find("helper").unwrap();
    report(
        cx,
        &path,
        json!([{
            "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}},
            "severity": 1,
            "message": "cannot find function `helper`",
        }]),
    );
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(name + 2..name + 2, cx);
    });

    // The keys: at the cursor, with the problem above it.
    press(cx, window, "secondary-i");
    wait_until(cx, |cx| hover_shown(cx, &workspace).is_some());
    assert_eq!(
        hover_shown(cx, &workspace),
        Some((
            Some("```rust\nfn helper()\n```\n\nHelps.".to_string()),
            vec!["cannot find function `helper`".to_string()]
        ))
    );
    {
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| window.render_frame(cx));
        assert!(vcx.debug_bounds("code-hover").is_some(), "the panel shows");
    }
    // Moving the cursor puts it away; so does Esc.
    press(cx, window, "right");
    assert_eq!(hover_shown(cx, &workspace), None);
    press(cx, window, "secondary-i");
    wait_until(cx, |cx| hover_shown(cx, &workspace).is_some());
    press(cx, window, "escape");
    assert_eq!(hover_shown(cx, &workspace), None);

    // The pointer: asked once it has rested on the name.
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let at = vcx.update(|_, cx| {
        let state = workspace.read(cx).editor().state().read(cx);
        state
            .range_to_bounds(&(name + 1..name + 2))
            .unwrap()
            .center()
    });
    vcx.simulate_mouse_move(at, None, gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(hover_shown(&mut vcx, &workspace), None, "not at once");
    vcx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    wait_until(&mut vcx, |cx| hover_shown(cx, &workspace).is_some());
    let shown = hover_shown(&mut vcx, &workspace).expect("after resting");
    assert!(shown.0.unwrap().contains("Helps."));
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("code-hover").is_some());
    assert!(
        vcx.debug_bounds("problem").is_none(),
        "the problem shows in the hover, not on its own"
    );

    // The palette covers it.
    vcx.update(|window, cx| {
        window.press("secondary-k", cx);
        window.render_frame(cx);
    });
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("code-hover").is_none());
    vcx.update(|window, cx| window.press("escape", cx));
    vcx.run_until_parked();

    // Typing puts it away.
    vcx.update(|window, cx| {
        window.render_frame(cx);
        window.input("x", cx);
    });
    vcx.run_until_parked();
    assert_eq!(hover_shown(&mut vcx, &workspace), None);
}

#[gpui_kit::test]
fn hover_without_a_server_shows_nothing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "helper here\n").unwrap();
    let (window, workspace) = open(cx, &path);
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(2..2, cx);
    });
    press(cx, window, "secondary-i");
    assert_eq!(hover_shown(cx, &workspace), None);
    type_slowly(cx, window, "(");
    cx.update(|cx| {
        assert!(
            workspace
                .read(cx)
                .tab()
                .hover
                .read(cx)
                .signature()
                .is_none()
        )
    });
}

#[gpui_kit::test]
fn signature_help_follows_the_call(cx: &mut TestAppContext) {
    use serde_json::json;

    let text = "fn main() {\n    \n}\nfn f(a: u32, b: &str) {}\n";
    let (window, workspace, _path, _dir) = with_fake_server(cx, text, |method, params| {
        match method {
            "textDocument/signatureHelp" => {
                // The second parameter once past the comma, as a server
                // works it out from the text.
                let past_comma = params["position"]["character"].as_u64() > Some(7);
                json!({
                    "signatures": [{
                        "label": "fn f(a: u32, b: &str)",
                        "parameters": [{"label": "a: u32"}, {"label": "b: &str"}],
                    }],
                    "activeSignature": 0,
                    "activeParameter": if past_comma { 1 } else { 0 },
                })
            }
            _ => serde_json::Value::Null,
        }
    });
    let signature = |cx: &mut TestAppContext| {
        cx.update(|cx| workspace.read(cx).tab().hover.read(cx).signature())
    };
    let active = |cx: &mut TestAppContext| signature(cx).and_then(|(_, active)| active);
    let inside = text.find("\n}").unwrap();
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(inside..inside, cx);
    });

    type_slowly(cx, window, "f(");
    wait_until(cx, |cx| signature(cx).is_some());
    assert_eq!(
        signature(cx),
        Some((
            "fn f(a: u32, b: &str)".to_string(),
            Some("a: u32".to_string())
        ))
    );
    {
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| window.render_frame(cx));
        assert!(vcx.debug_bounds("signature-help").is_some());
    }
    type_slowly(cx, window, "1,");
    wait_until(cx, |cx| active(cx).as_deref() == Some("b: &str"));
    assert_eq!(active(cx).as_deref(), Some("b: &str"));

    // Leaving the call puts it away.
    press(cx, window, "home");
    assert_eq!(signature(cx), None);

    // So does Esc, and `)`.
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        let end = editor.text(cx).find("1,").unwrap() + 2;
        editor.select(end..end, cx);
    });
    type_slowly(cx, window, ",");
    wait_until(cx, |cx| signature(cx).is_some());
    press(cx, window, "escape");
    assert_eq!(signature(cx), None);
    type_slowly(cx, window, ",");
    wait_until(cx, |cx| signature(cx).is_some());
    type_slowly(cx, window, ")");
    assert_eq!(signature(cx), None);
}
