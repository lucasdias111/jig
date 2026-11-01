//! The hover and signature help, with and without a server.

use super::*;

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
    let (client, _seen) = jig_lsp::lsp::fake_server(dir.path().to_path_buf(), answer);
    let server = jig_lsp::lsp::server_for("rust").unwrap();
    let root = jig_lsp::lsp::root_for(server, &path);
    cx.update(|cx| jig_lsp::lsp::register(&server.name, root, client, cx));
    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &path);
    (window, workspace, path, dir)
}

pub(super) type HoverShown = Option<(Option<String>, Vec<String>)>;

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
fn hover_goes_once_the_pointer_leaves_the_code(cx: &mut TestAppContext) {
    use serde_json::json;

    let text = "fn main() {\n    helper();\n}\n";
    let (window, workspace, _path, _dir) = with_fake_server(cx, text, |method, _| match method {
        "textDocument/hover" => json!({
            "contents": {"kind": "markdown", "value": "Helps."},
            "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}},
        }),
        _ => serde_json::Value::Null,
    });
    let name = text.find("helper").unwrap();
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
    vcx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    wait_until(&mut vcx, |cx| hover_shown(cx, &workspace).is_some());

    // Onto empty code: it goes after a moment.
    let blank = gpui_kit::point(at.x + gpui_kit::px(200.), at.y);
    vcx.simulate_mouse_move(blank, None, gpui_kit::Modifiers::none());
    vcx.executor()
        .advance_clock(std::time::Duration::from_millis(1500));
    vcx.run_until_parked();
    assert_eq!(hover_shown(&mut vcx, &workspace), None);

    vcx.simulate_mouse_move(at, None, gpui_kit::Modifiers::none());
    vcx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    wait_until(&mut vcx, |cx| hover_shown(cx, &workspace).is_some());

    // Off the code, onto the file tree beside it: no Esc needed.
    vcx.simulate_mouse_move(
        gpui_kit::point(gpui_kit::px(4.), at.y),
        None,
        gpui_kit::Modifiers::none(),
    );
    vcx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    vcx.run_until_parked();
    assert_eq!(hover_shown(&mut vcx, &workspace), None);
}

#[gpui_kit::test]
fn hover_shows_over_the_agent_conversation(cx: &mut TestAppContext) {
    use serde_json::json;

    let text = "fn main() {\n    helper();\n}\n";
    let (window, workspace, _path, _dir) = with_fake_server(cx, text, |method, _| match method {
        "textDocument/hover" => json!({
            "contents": {"kind": "markdown", "value": "Helps."},
            "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}},
        }),
        _ => serde_json::Value::Null,
    });
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            // Beside the code, below the name: where the panel would reach.
            this.chat.as_mut().unwrap().anchor = gpui_kit::point(px(360.), px(260.));
        })
    });
    let name = text.find("helper").unwrap();
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("agent-chat-header").is_some());
    let at = vcx.update(|_, cx| {
        let state = workspace.read(cx).editor().state().read(cx);
        state
            .range_to_bounds(&(name + 1..name + 2))
            .unwrap()
            .center()
    });
    vcx.simulate_mouse_move(at, None, gpui_kit::Modifiers::none());
    vcx.executor()
        .advance_clock(std::time::Duration::from_millis(600));
    wait_until(&mut vcx, |cx| hover_shown(cx, &workspace).is_some());
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("code-hover").is_some(), "the panel shows");
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
