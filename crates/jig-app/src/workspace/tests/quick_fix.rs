//! Quick Fix: the server's fixes and Fix with Agent, in the list and the
//! problem's panel.

use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

use super::*;

const TEXT: &str = "fn main() {\n    let x: u32 = \"s\";\n}\n";

/// `TEXT` in a Rust project whose server offers one fix for the problem on
/// its second line, which is reported.
fn with_a_fix(
    cx: &mut TestAppContext,
) -> (
    AnyWindowHandle,
    Entity<Workspace>,
    mpsc::Receiver<Value>,
    tempfile::TempDir,
) {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let path = dir.path().join("src/main.rs");
    std::fs::write(&path, TEXT).unwrap();
    let uri = crate::lsp::file_uri(&path);
    let (client, seen) = crate::lsp::fake_server(dir.path().to_path_buf(), move |method, _| {
        match method {
            "textDocument/codeAction" => json!([
                {"title": "Run a command", "command": "x.run"},
                {
                    "title": "Use a number",
                    "kind": "quickfix",
                    "isPreferred": true,
                    "edit": {"changes": {uri.clone(): [{
                        "range": {"start": {"line": 1, "character": 17}, "end": {"line": 1, "character": 20}},
                        "newText": "1",
                    }]}},
                },
            ]),
            _ => Value::Null,
        }
    });
    let server = crate::lsp::server_for("rust").unwrap();
    let root = crate::lsp::root_for(server, &path);
    cx.update(|cx| crate::lsp::register(&server.name, root, client, cx));
    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &path);

    let published = crate::lsp::diagnostics::Published {
        key: crate::lsp::diagnostics::path_key(&path),
        version: None,
        items: serde_json::from_value(json!([{
            "range": {"start": {"line": 1, "character": 17}, "end": {"line": 1, "character": 20}},
            "severity": 1,
            "source": "rustc",
            "code": "E0308",
            "message": "mismatched types",
        }]))
        .unwrap(),
        encoding: crate::lsp::Encoding::Utf16,
    };
    cx.update(|cx| crate::lsp::diagnostics::publish(published, cx));
    cx.run_until_parked();
    (window, workspace, seen, dir)
}

fn listed(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Option<Vec<String>> {
    cx.update(|cx| {
        let state = workspace.read(cx).editor().state().read(cx);
        let menu = state.code_action_menu_state();
        menu.open.then(|| {
            menu.items
                .iter()
                .map(|item| item.action.title.clone())
                .collect()
        })
    })
}

/// What the conversation Fix with Agent started sent the agent.
fn sent(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Option<String> {
    cx.update(|cx| workspace.read(cx).agent_prompt())
}

fn open_quick_fix(cx: &mut TestAppContext, window: AnyWindowHandle, workspace: &Entity<Workspace>) {
    let on_line = TEXT.find("let").unwrap();
    cx.update(|cx| {
        workspace
            .read(cx)
            .editor()
            .clone()
            .select(on_line..on_line, cx)
    });
    press(cx, window, "secondary-.");
    wait_until(cx, |cx| listed(cx, workspace).is_some());
}

#[gpui_kit::test]
fn quick_fix_lists_the_servers_fixes_then_the_agent(cx: &mut TestAppContext) {
    let (window, workspace, seen, _dir) = with_a_fix(cx);
    open_quick_fix(cx, window, &workspace);
    assert_eq!(
        listed(cx, &workspace).unwrap(),
        ["Use a number", "Fix with Agent"]
    );
    // The server was told which problem, over the whole line.
    let asked = seen
        .try_iter()
        .find(|message| message["method"] == "textDocument/codeAction")
        .expect("asked for fixes");
    assert_eq!(
        asked["params"]["context"]["diagnostics"][0]["message"],
        "mismatched types"
    );
    assert_eq!(asked["params"]["range"]["start"]["character"], 0);
    assert_eq!(asked["params"]["context"]["only"], json!(["quickfix"]));

    // The server's fix lands in the buffer, unsaved.
    press(cx, window, "enter");
    wait_until(cx, |cx| text(cx, &workspace).contains("u32 = 1;"));
    assert_eq!(
        text(cx, &workspace),
        "fn main() {\n    let x: u32 = 1;\n}\n"
    );
    assert!(cx.update(|cx| workspace.read(cx).tab().dirty));
}

#[gpui_kit::test]
fn fix_with_agent_sends_the_problem(cx: &mut TestAppContext) {
    let (window, workspace, _seen, _dir) = with_a_fix(cx);
    open_quick_fix(cx, window, &workspace);
    press(cx, window, "down");
    press(cx, window, "enter");
    wait_until(cx, |cx| sent(cx, &workspace).is_some());
    let sent = sent(cx, &workspace).unwrap();
    assert!(
        sent.contains("line 2, error: mismatched types (rustc E0308)"),
        "{sent}"
    );
}

#[gpui_kit::test]
fn nothing_to_fix_says_so(cx: &mut TestAppContext) {
    let (window, workspace, _seen, _dir) = with_a_fix(cx);
    cx.update(|cx| workspace.read(cx).editor().clone().select(0..0, cx));
    press(cx, window, "secondary-.");
    wait_until(cx, |cx| note(cx, &workspace).is_some());
    assert!(matches!(
        note(cx, &workspace),
        Some(Bubble::Message(message)) if message == "No problems on this line."
    ));
    assert_eq!(listed(cx, &workspace), None);
}

#[gpui_kit::test]
fn problem_panel_offers_fixes_and_waits_for_the_pointer(cx: &mut TestAppContext) {
    let (window, workspace, _seen, _dir) = with_a_fix(cx);
    let message = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let hover = workspace.read(cx).tab().problems.hover.clone();
            hover.update(cx, |hover, cx| hover.message(cx))
        })
    };
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let squiggle = TEXT.find("\"s\"").unwrap();
    let (on_problem, below) = vcx.update(|_, cx| {
        let state = workspace.read(cx).editor().state().read(cx);
        let at = state
            .range_to_bounds(&(squiggle + 1..squiggle + 2))
            .unwrap();
        let end = state.range_to_bounds(&(0..1)).unwrap();
        (at.center(), end.center())
    });
    vcx.simulate_mouse_move(on_problem, None, gpui_kit::Modifiers::none());
    wait_until(&mut vcx, |cx| message(cx).is_some());
    wait_until(&mut vcx, |cx| {
        cx.update(|cx| {
            let hover = workspace.read(cx).tab().problems.hover.clone();
            hover.read(cx).fix_titles(cx) == ["Use a number"]
        })
    });
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("fix-with-agent").is_some());

    // Off the problem, it stays a moment, then goes.
    vcx.simulate_mouse_move(below, None, gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert!(message(&mut vcx).is_some(), "lingers");
    vcx.executor().advance_clock(Duration::from_millis(500));
    vcx.run_until_parked();
    assert_eq!(message(&mut vcx), None);

    // Onto the panel, it stays while the pointer does, and its buttons work.
    vcx.simulate_mouse_move(on_problem, None, gpui_kit::Modifiers::none());
    wait_until(&mut vcx, |cx| message(cx).is_some());
    vcx.update(|window, cx| window.render_frame(cx));
    let button = vcx.debug_bounds("fix-with-agent").unwrap().center();
    vcx.simulate_mouse_move(button, None, gpui_kit::Modifiers::none());
    vcx.executor().advance_clock(Duration::from_millis(1000));
    vcx.run_until_parked();
    assert!(message(&mut vcx).is_some(), "held");
    vcx.simulate_click(button, gpui_kit::Modifiers::none());
    wait_until(&mut vcx, |cx| sent(cx, &workspace).is_some());
    assert!(
        sent(&mut vcx, &workspace)
            .unwrap()
            .contains("mismatched types")
    );
}
