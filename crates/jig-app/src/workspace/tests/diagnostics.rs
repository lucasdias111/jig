//! Problems, hover and signature help.

use super::*;

mod hover;

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
    let published = jig_lsp::lsp::diagnostics::Published {
        key: jig_lsp::lsp::diagnostics::path_key(path),
        version: None,
        items: serde_json::from_value(items).unwrap(),
        encoding: jig_lsp::lsp::Encoding::Utf16,
    };
    cx.update(|cx| jig_lsp::lsp::diagnostics::publish(published, cx));
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
        jig_lsp::lsp::fake_server(dir.path().to_path_buf(), |_, _| serde_json::Value::Null);
    let server = jig_lsp::lsp::server_for("rust").unwrap();
    let root = jig_lsp::lsp::root_for(server, &path);
    cx.update(|cx| jig_lsp::lsp::register(&server.name, root, client, cx));
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
fn a_fix_sends_the_problems_on_the_line(cx: &mut TestAppContext) {
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    let text = "fn main() {\n    let x: u32 = \"s\";\n}\n";
    std::fs::write(&path, text).unwrap();
    let (window, workspace) = open(cx, &path);
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
    let ask = |diagnostics: bool, cx: &mut TestAppContext| {
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| {
                this.editor().select(on_line..on_line, cx);
                let ask = super::super::commands::Ask {
                    diagnostics,
                    ..super::super::commands::Ask::new("Fix it.")
                };
                this.start_conversation(ask, window, cx);
            })
        });
        cx.update(|cx| workspace.read(cx).agent_prompt()).unwrap()
    };
    let sent = ask(true, cx);
    assert!(
        sent.contains(
            "The language server reports:\n- line 2, error: mismatched types (rustc E0308)"
        ),
        "{sent}"
    );
    // Asked without them, they don't go.
    let sent = ask(false, cx);
    assert!(!sent.contains("language server"), "{sent}");
}
