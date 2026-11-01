//! Run configurations, breakpoints and the debugger.

use super::*;

mod breakpoints;
mod languages;
mod running;

/// Plays `lldb-dap`'s part, as it answered for a real program: stops at
/// the breakpoint, shows `numbers` and `total`, and finishes on resume.
/// Records the breakpoint lines it was given.
pub(super) fn fake_adapter(
    source: std::path::PathBuf,
    breakpoints: std::sync::Arc<std::sync::Mutex<Vec<u64>>>,
) {
    use jig_debug::dap::{Client, frame, read_message, socket_pair};
    use serde_json::json;
    crate::workspace::debug::FAKE_ADAPTER.with(|fake| {
            *fake.borrow_mut() = Some(Box::new(move |messages| {
                let (ours, theirs) = socket_pair();
                let source = source.clone();
                let breakpoints = breakpoints.clone();
                std::thread::spawn(move || {
                    let mut reader = std::io::BufReader::new(theirs.try_clone().unwrap());
                    let mut out = theirs;
                    let mut send = |message: serde_json::Value| {
                        use std::io::Write as _;
                        out.write_all(&frame(&message)).unwrap();
                    };
                    while let Some(request) = read_message(&mut reader) {
                        let command = request["command"].as_str().unwrap_or_default().to_string();
                        let respond = |body: serde_json::Value| {
                            json!({"type": "response", "request_seq": request["seq"],
                                   "command": command, "success": true, "body": body})
                        };
                        match command.as_str() {
                            "initialize" => {
                                send(respond(json!({})));
                                send(json!({"type": "event", "event": "initialized"}));
                            }
                            "setBreakpoints" => {
                                for bp in request["arguments"]["breakpoints"].as_array().unwrap() {
                                    breakpoints.lock().unwrap().push(bp["line"].as_u64().unwrap());
                                }
                                send(respond(json!({"breakpoints": []})));
                            }
                            "configurationDone" => {
                                send(respond(json!({})));
                                send(json!({"type": "event", "event": "stopped",
                                            "body": {"reason": "breakpoint", "threadId": 1}}));
                            }
                            "stackTrace" => send(respond(json!({"stackFrames": [
                                {"id": 10, "name": "app::main", "line": 3, "column": 5,
                                 "source": {"path": source}},
                                {"id": 11, "name": "std::rt::lang_start", "line": 1, "column": 1},
                            ]}))),
                            "scopes" => send(respond(json!({"scopes": [
                                {"name": "Locals", "variablesReference": 100, "expensive": false},
                                {"name": "Registers", "variablesReference": 200, "expensive": true},
                            ]}))),
                            "variables" => {
                                let variables = match request["arguments"]["variablesReference"].as_i64() {
                                    Some(100) => json!([
                                        {"name": "numbers", "value": "size=3", "type": "Vec<i32>",
                                         "variablesReference": 101},
                                        {"name": "total", "value": "6", "type": "i32",
                                         "variablesReference": 0},
                                    ]),
                                    _ => json!([
                                        {"name": "[0]", "value": "1", "variablesReference": 0},
                                        {"name": "[1]", "value": "2", "variablesReference": 0},
                                    ]),
                                };
                                send(respond(json!({"variables": variables})));
                            }
                            "continue" => {
                                send(respond(json!({"allThreadsContinued": true})));
                                send(json!({"type": "event", "event": "output",
                                            "body": {"category": "stdout", "output": "total 6\n"}}));
                                send(json!({"type": "event", "event": "exited",
                                            "body": {"exitCode": 0}}));
                                send(json!({"type": "event", "event": "terminated"}));
                            }
                            "disconnect" => {
                                send(respond(json!({})));
                                return;
                            }
                            _ => send(respond(json!({}))),
                        }
                    }
                });
                Client::connect(0, ours.try_clone().unwrap(), ours, messages)
            }));
        });
}

#[gpui_kit::test]
fn debugs_to_a_breakpoint_shows_variables_and_resumes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("src/main.rs");
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::create_dir_all(dir.path().join(".jig")).unwrap();
    std::fs::write(
            &source,
            "fn main() {\n    let numbers = vec![1, 2, 3];\n    let total: i32 = numbers.iter().sum();\n    println!(\"total {total}\");\n}\n",
        )
        .unwrap();
    std::fs::write(
        dir.path().join(".jig/run.toml"),
        "[[run]]\nname = \"App\"\ncommand = \"./app\"\nprogram = \"app\"\n",
    )
    .unwrap();
    let source = source.canonicalize().unwrap();
    let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    fake_adapter(source.clone(), sent.clone());
    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &source);

    // ⌘F8 on the third line.
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        ws.update(cx, |this, cx| {
            this.editor()
                .state()
                .update(cx, |s, cx| s.set_selected_range(50..50, cx))
        });
        window.press("secondary-f8", cx);
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert_eq!(this.breakpoint_lines(this.active, cx), [2].into());
    });

    step(cx, window, |window, cx| window.press("ctrl-d", cx));
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).run_picker_rows(cx))
            .is_some_and(|rows| rows.first().is_some_and(|row| row == "App"))
    });
    step(cx, window, |window, cx| window.press("enter", cx));
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).debug_variable_rows().len() >= 3)
    });
    assert_eq!(
        *sent.lock().unwrap(),
        [3],
        "1-based lines go to the adapter"
    );
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert_eq!(
            this.debug_phase(),
            Some(crate::workspace::debug::Phase::Paused)
        );
        assert_eq!(this.debug_frames(), ["app::main", "std::rt::lang_start"]);
        assert_eq!(this.execution_line(cx), Some((source.clone(), 2)));
        assert_eq!(
            this.debug_variable_rows(),
            [
                (0, "Locals".into(), "".into()),
                (1, "numbers".into(), "size=3".into()),
                (1, "total".into(), "6".into()),
                (0, "Registers".into(), "".into()),
            ]
        );
    });

    let ws = workspace.clone();
    step(cx, window, move |_, cx| {
        ws.update(cx, |this, cx| this.expand_variable("numbers", cx))
    });
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).debug_variable_rows().len() == 6)
    });
    assert_eq!(
        cx.update(|cx| workspace.read(cx).debug_variable_rows())[2],
        (2, "[0]".to_string(), "1".to_string())
    );

    step(cx, window, |window, cx| window.press("f9", cx));
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).run_output(cx))
            .is_some_and(|(_, _, ending)| ending.is_some())
    });
    let (_, lines, ending) = cx.update(|cx| workspace.read(cx).run_output(cx)).unwrap();
    assert_eq!(ending.as_deref(), Some("Debugging finished"));
    let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
    assert!(texts.contains(&"total 6".to_string()), "{texts:?}");
    assert!(texts.contains(&"Process finished with exit code 0".to_string()));
    cx.update(|cx| assert_eq!(workspace.read(cx).execution_line(cx), None));
}

/// Debugs a real Cargo project with the real `lldb-dap`: builds it,
/// stops at a breakpoint, reads a variable, resumes. Needs Xcode's tools
/// and Cargo, and macOS may ask once for permission to debug.
#[gpui_kit::test]
#[ignore]
fn debug_live(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let source = root.join("src/main.rs");
    std::fs::write(
            &source,
            "fn main() {\n    let numbers = vec![1, 2, 3];\n    let total: i32 = numbers.iter().sum();\n    println!(\"total {total}\");\n}\n",
        )
        .unwrap();
    let (window, workspace) = open(cx, &root);
    open_file(cx, window, &workspace, &source);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        ws.update(cx, |this, cx| {
            this.editor()
                .state()
                .update(cx, |s, cx| s.set_selected_range(100..100, cx))
        });
        window.press("secondary-f8", cx);
    });
    step(cx, window, |window, cx| window.press("ctrl-d", cx));
    let wait = |cx: &mut TestAppContext, done: &dyn Fn(&Workspace, &gpui_kit::App) -> bool| {
        for _ in 0..1200 {
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(50));
            cx.run_until_parked();
            if cx.update(|cx| done(workspace.read(cx), cx)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let output = cx.update(|cx| workspace.read(cx).run_output(cx));
        panic!("timed out: {output:#?}");
    };
    wait(cx, &|this, cx| {
        this.run_picker_rows(cx)
            .is_some_and(|rows| rows.iter().any(|row| row == "sample"))
    });
    step(cx, window, |window, cx| window.input("sample", cx));
    step(cx, window, |window, cx| window.press("enter", cx));
    wait(cx, &|this, _| {
        this.debug_variable_rows()
            .iter()
            .any(|(_, name, _)| name == "total")
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert_eq!(this.execution_line(cx), Some((source.clone(), 3)));
        let rows = this.debug_variable_rows();
        eprintln!("{rows:#?}");
        assert!(rows.contains(&(1, "total".into(), "6".into())));
        assert!(rows.contains(&(1, "numbers".into(), "size=3".into())));
    });
    // A `println!` line can have more than one breakpoint location, so
    // resume until it ends.
    for _ in 0..5 {
        step(cx, window, |window, cx| window.press("f9", cx));
        wait(cx, &|this, app| {
            this.run_output(app)
                .is_some_and(|(_, _, ending)| ending.is_some())
                || this.debug_phase() == Some(crate::workspace::debug::Phase::Paused)
        });
        if cx.update(|cx| workspace.read(cx).debug_phase())
            == Some(crate::workspace::debug::Phase::Ended)
        {
            break;
        }
    }
    let (_, lines, _) = cx.update(|cx| workspace.read(cx).run_output(cx)).unwrap();
    let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
    eprintln!("{texts:#?}");
    assert!(texts.contains(&"total 6".to_string()));
}

/// Pausing where a local refers to a struct whose fields reach a great
/// deal more (as an app's state does) still shows the locals and
/// resumes: Rust's LLDB formatters would sum all of it up first, and LLDB
/// answers nothing else meanwhile. Needs what `debug_live` needs.
#[gpui_kit::test]
#[ignore]
fn resumes_past_a_big_struct_live(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let source = root.join("src/main.rs");
    // Each level refers to the next twice: 2^40 fields to sum up.
    std::fs::write(
        &source,
        "#[allow(dead_code)]\nstruct Node {\n    left: Option<&'static Node>,\n    right: Option<&'static Node>,\n}\n\nfn main() {\n    let mut node: &'static Node = Box::leak(Box::new(Node { left: None, right: None }));\n    for _ in 0..40 {\n        node = Box::leak(Box::new(Node { left: Some(node), right: Some(node) }));\n    }\n    visit(node);\n}\n\nfn visit(node: &Node) {\n    println!(\"visited {}\", node.left.is_some());\n}\n",
    )
    .unwrap();
    let (window, workspace) = open(cx, &root);
    open_file(cx, window, &workspace, &source);
    step(cx, window, |window, cx| {
        workspace.update(cx, |this, cx| {
            let id = this.tab().id();
            this.toggle_breakpoint_line(id, 15, window, cx);
        })
    });
    let wait = |cx: &mut TestAppContext, done: &dyn Fn(&Workspace, &gpui_kit::App) -> bool| {
        for _ in 0..600 {
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(50));
            cx.run_until_parked();
            if cx.update(|cx| done(workspace.read(cx), cx)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let output = cx.update(|cx| workspace.read(cx).run_output(cx));
        panic!("timed out: {output:#?}");
    };
    use crate::workspace::debug::Phase;
    step(cx, window, |window, cx| window.press("ctrl-d", cx));
    wait(cx, &|this, cx| {
        this.run_picker_rows(cx)
            .is_some_and(|rows| rows.iter().any(|row| row == "sample"))
    });
    step(cx, window, |window, cx| window.input("sample", cx));
    step(cx, window, |window, cx| window.press("enter", cx));
    wait(cx, &|this, _| {
        this.debug_variable_rows()
            .iter()
            .any(|(_, name, _)| name == "node")
    });
    // A `println!` line can have more than one breakpoint location.
    for _ in 0..5 {
        step(cx, window, |window, cx| window.press("f9", cx));
        wait(cx, &|this, _| {
            matches!(this.debug_phase(), Some(Phase::Paused) | Some(Phase::Ended))
        });
        if cx.update(|cx| workspace.read(cx).debug_phase()) == Some(Phase::Ended) {
            break;
        }
    }
    let (_, lines, _) = cx.update(|cx| workspace.read(cx).run_output(cx)).unwrap();
    let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
    assert!(texts.contains(&"visited true".to_string()), "{texts:#?}");
}
