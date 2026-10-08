//! Run configurations, breakpoints and the debugger.

use super::*;

#[gpui_kit::test]
fn runs_a_configuration_and_opens_files_from_its_output(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::create_dir_all(dir.path().join(".jig")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), ORIGINAL).unwrap();
    let hello = if cfg!(windows) {
        "echo hello %GREETING%& 1>&2 echo src/lib.rs:1:4: here& exit 2"
    } else {
        "echo hello $GREETING; echo 'src/lib.rs:1:4: here' >&2; exit 2"
    };
    std::fs::write(
        dir.path().join(".jig/run.toml"),
        format!(
            r#"
[[run]]
name = "Other"
command = "exit 0"

[[run]]
name = "Hello"
command = "{hello}"
env = {{ GREETING = "there" }}
"#
        ),
    )
    .unwrap();
    let (window, workspace) = open(cx, dir.path());

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("ctrl-alt-r", cx);
    });
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).run_picker_rows(cx))
            .is_some_and(|rows| rows.len() == 3)
    });
    assert_eq!(
        cx.update(|cx| workspace.read(cx).run_picker_rows(cx))
            .unwrap(),
        ["Other", "Hello", "Edit run.toml…"]
    );
    step(cx, window, |window, cx| window.input("hel", cx));
    step(cx, window, |window, cx| window.press("enter", cx));
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).run_output())
            .is_some_and(|(_, _, ending)| ending.is_some())
    });
    let (first, lines, ending) = cx.update(|cx| workspace.read(cx).run_output()).unwrap();
    assert_eq!(ending.as_deref(), Some("Process finished with exit code 2"));
    let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
    assert!(texts[0].starts_with("$ echo hello"));
    assert!(texts.contains(&"hello there".to_string()), "{texts:?}");
    assert!(
        texts
            .last()
            .unwrap()
            .starts_with("Process finished with exit code 2 (")
    );

    let link = lines
        .iter()
        .find_map(|line| line.links.first().cloned())
        .expect("the file reference is a link");
    let target = workspace.clone();
    step(cx, window, move |window, cx| {
        target.update(cx, |this, cx| this.open_link(&link, window, cx));
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
        assert_eq!(this.editor().selection(cx), 3..3);
    });

    // ⌃R runs the chosen configuration again.
    step(cx, window, |window, cx| window.press("ctrl-r", cx));
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).run_output())
            .is_some_and(|(id, _, ending)| id != first && ending.is_some())
    });
}

/// Plays `lldb-dap`'s part, as it answered for a real program: stops at
/// the breakpoint, shows `numbers` and `total`, and finishes on resume.
/// Records the breakpoint lines it was given.
fn fake_adapter(
    source: std::path::PathBuf,
    breakpoints: std::sync::Arc<std::sync::Mutex<Vec<u64>>>,
) {
    use crate::dap::{Client, frame, read_message, socket_pair};
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
        cx.update(|cx| workspace.read(cx).run_output())
            .is_some_and(|(_, _, ending)| ending.is_some())
    });
    let (_, lines, ending) = cx.update(|cx| workspace.read(cx).run_output()).unwrap();
    assert_eq!(ending.as_deref(), Some("Debugging finished"));
    let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
    assert!(texts.contains(&"total 6".to_string()), "{texts:?}");
    assert!(texts.contains(&"Process finished with exit code 0".to_string()));
    cx.update(|cx| assert_eq!(workspace.read(cx).execution_line(cx), None));
}

#[gpui_kit::test]
fn clicking_the_gutter_toggles_a_breakpoint(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            let editor = this.editor().clone();
            editor.apply_edit(0..0, "one\ntwo\nthree\nfour\n", window, cx);
        });
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let (bounds, line_height) = vcx.update(|_, cx| {
        let state = workspace.read(cx).editor().state().read(cx);
        (state.input_bounds(), state.line_height().unwrap())
    });
    // The line numbers start at the editor's left edge.
    let third_line = gpui_kit::point(
        bounds.origin.x + gpui_kit::px(6.),
        bounds.origin.y + line_height * 2.5,
    );
    let lines = |vcx: &mut gpui_kit::VisualTestContext| {
        vcx.update(|_, cx| {
            let this = workspace.read(cx);
            this.breakpoint_lines(this.active, cx)
        })
    };
    let selection = |vcx: &mut gpui_kit::VisualTestContext| {
        vcx.update(|_, cx| workspace.read(cx).editor().selection(cx))
    };
    let before = selection(&mut vcx);
    vcx.simulate_click(third_line, gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(lines(&mut vcx), [2].into());
    assert_eq!(
        selection(&mut vcx),
        before,
        "the click doesn't move the cursor"
    );
    vcx.update(|window, cx| window.render_frame(cx));
    vcx.simulate_click(third_line, gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert!(lines(&mut vcx).is_empty());
}

#[gpui_kit::test]
fn the_title_bar_debug_button_opens_the_picker(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let button = vcx
        .debug_bounds("debug-selected")
        .expect("the button is in the title bar");
    vcx.simulate_click(button.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| workspace.read(cx).run_picker_rows(cx).is_some()));
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
        let output = cx.update(|cx| workspace.read(cx).run_output());
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
        wait(cx, &|this, _| {
            this.run_output()
                .is_some_and(|(_, _, ending)| ending.is_some())
                || this.debug_phase() == Some(crate::workspace::debug::Phase::Paused)
        });
        if cx.update(|cx| workspace.read(cx).debug_phase())
            == Some(crate::workspace::debug::Phase::Ended)
        {
            break;
        }
    }
    let (_, lines, _) = cx.update(|cx| workspace.read(cx).run_output()).unwrap();
    let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
    eprintln!("{texts:#?}");
    assert!(texts.contains(&"total 6".to_string()));
}

#[gpui_kit::test]
fn breakpoints_follow_the_debugging_setting(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.ts");
    std::fs::write(&path, "const a = 1;\nconst b = 2;\n").unwrap();
    let (window, workspace) = open(cx, &path);
    let lines = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let this = workspace.read(cx);
            this.breakpoint_lines(this.active, cx)
        })
    };
    let toggle = |cx: &mut TestAppContext| {
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-f8", cx);
        })
    };

    // TypeScript's debugger is off to begin with: no breakpoints.
    toggle(cx);
    assert!(lines(cx).is_empty());

    step(cx, window, |_, cx| {
        crate::settings::update(cx, |s| s.debugging.set_enabled("typescript", true))
    });
    toggle(cx);
    assert_eq!(lines(cx), [0].into());

    // Off again hides it, and on again brings it back.
    step(cx, window, |_, cx| {
        crate::settings::update(cx, |s| s.debugging.set_enabled("typescript", false))
    });
    assert!(lines(cx).is_empty());
    step(cx, window, |_, cx| {
        crate::settings::update(cx, |s| s.debugging.set_enabled("typescript", true))
    });
    assert_eq!(lines(cx), [0].into());
}

#[gpui_kit::test]
fn debugging_a_language_that_is_off_starts_nothing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".jig")).unwrap();
    std::fs::write(dir.path().join("app.ts"), "console.log(1);\n").unwrap();
    std::fs::write(
        dir.path().join(".jig/run.toml"),
        "[[run]]\nname = \"App\"\ncommand = \"node app.ts\"\n",
    )
    .unwrap();
    let (window, workspace) = open(cx, dir.path());
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("ctrl-d", cx);
    });
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).run_picker_rows(cx))
            .is_some_and(|rows| rows.first().is_some_and(|row| row == "App"))
    });
    step(cx, window, |window, cx| window.press("enter", cx));
    assert!(cx.update(|cx| workspace.read(cx).run_output()).is_none());
}

/// Debugs `source` in a fresh project with a real debugger, found in
/// `JIG_DEBUGGERS_DIR`: run `command` under ⌃D with a breakpoint on
/// `line` (0-based), check `variable` shows `value`, resume, and check
/// the program printed `printed`.
#[allow(clippy::too_many_arguments)]
fn debug_project_live(
    cx: &mut TestAppContext,
    debugger: &'static str,
    files: &[(&str, &str)],
    source: &str,
    command: &str,
    line: u32,
    variable: &str,
    value: &str,
    printed: &str,
) {
    assert!(
        std::env::var_os("JIG_DEBUGGERS_DIR").is_some(),
        "JIG_DEBUGGERS_DIR: where the debuggers are installed"
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".jig")).unwrap();
    for (name, text) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let source = root.join(source);
    std::fs::write(
        root.join(".jig/run.toml"),
        format!("[[run]]\nname = \"App\"\ncommand = \"{command}\"\n"),
    )
    .unwrap();
    let (window, workspace) = open(cx, &root);
    step(cx, window, move |_, cx| {
        crate::settings::update(cx, |s| s.debugging.set_enabled(debugger, true))
    });
    open_file(cx, window, &workspace, &source);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        ws.update(cx, |this, cx| {
            let text = this.editor().text(cx);
            let offset: usize = text
                .split_inclusive('\n')
                .take(line as usize)
                .map(str::len)
                .sum();
            this.editor()
                .state()
                .update(cx, |s, cx| s.set_selected_range(offset..offset, cx))
        });
        window.press("secondary-f8", cx);
    });
    step(cx, window, |window, cx| window.press("ctrl-d", cx));
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
        let output = cx.update(|cx| workspace.read(cx).run_output());
        panic!("timed out: {output:#?}");
    };
    wait(cx, &|this, cx| {
        this.run_picker_rows(cx)
            .is_some_and(|rows| rows.first().is_some_and(|row| row == "App"))
    });
    step(cx, window, |window, cx| window.press("enter", cx));
    let variable = variable.to_string();
    wait(cx, &|this, _| {
        this.debug_variable_rows()
            .iter()
            .any(|(_, name, _)| *name == variable)
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert_eq!(this.execution_line(cx), Some((source.clone(), line)));
        let rows = this.debug_variable_rows();
        eprintln!("{rows:#?}");
        // Some values carry an id that changes, as Java's `int[3]@8`.
        assert!(
            rows.iter().any(|(depth, name, shown)| {
                *depth == 1 && *name == variable && shown.starts_with(value)
            }),
            "{rows:#?}"
        );
    });
    step(cx, window, |window, cx| window.press("f9", cx));
    wait(cx, &|this, _| {
        this.run_output()
            .is_some_and(|(_, _, ending)| ending.is_some())
    });
    let (_, lines, _) = cx.update(|cx| workspace.read(cx).run_output()).unwrap();
    let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
    eprintln!("{texts:#?}");
    assert!(texts.contains(&printed.to_string()), "{texts:#?}");
}

/// TypeScript on Node with js-debug, through its child session. Needs
/// Node 23.6+. `JIG_TS_COMMAND="npm start"` tries the way package.json
/// scripts run.
#[gpui_kit::test]
#[ignore]
fn debug_typescript_live(cx: &mut TestAppContext) {
    let command = std::env::var("JIG_TS_COMMAND").unwrap_or_else(|_| "node main.ts".into());
    debug_project_live(
        cx,
        "typescript",
        &[
            (
                "main.ts",
                "function total(numbers: number[]): number {\n  return numbers.reduce((a, b) => a + b, 0);\n}\nconst numbers: number[] = [1, 2, 3];\nconst sum: number = total(numbers);\nconsole.log(`total ${sum}`);\n",
            ),
            (
                "package.json",
                r#"{"name": "sample", "scripts": {"start": "node main.ts"}}"#,
            ),
        ],
        "main.ts",
        &command,
        1,
        "numbers",
        "(3) [1, 2, 3]",
        "total 6",
    );
}

/// Python with debugpy.
#[gpui_kit::test]
#[ignore]
fn debug_python_live(cx: &mut TestAppContext) {
    debug_project_live(
        cx,
        "python",
        &[(
            "main.py",
            "def total(numbers):\n    return sum(numbers)\n\nnumbers = [1, 2, 3]\nresult = total(numbers)\nprint(f\"total {result}\")\n",
        )],
        "main.py",
        "python3 main.py",
        1,
        "numbers",
        "[1, 2, 3]",
        "total 6",
    );
}

/// Java with java-debug in jdtls, which finds the main class and builds
/// the project itself. Needs a JDK, Maven, and `JIG_LIVE_LSP=1` so the
/// real jdtls runs; `JIG_CACHE_DIR` keeps its index elsewhere.
#[gpui_kit::test]
#[ignore]
fn debug_java_live(cx: &mut TestAppContext) {
    assert!(
        std::env::var_os("JIG_LIVE_LSP").is_some(),
        "JIG_LIVE_LSP=1: Java's debugger lives in jdtls"
    );
    debug_project_live(
        cx,
        "java",
        &[
            (
                "pom.xml",
                "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n  <modelVersion>4.0.0</modelVersion>\n  <groupId>sample</groupId>\n  <artifactId>sample</artifactId>\n  <version>1.0</version>\n  <properties>\n    <maven.compiler.release>21</maven.compiler.release>\n  </properties>\n</project>\n",
            ),
            (
                "src/main/java/sample/Main.java",
                "package sample;\n\npublic class Main {\n    static int total(int[] numbers) {\n        int sum = 0;\n        for (int n : numbers) sum += n;\n        return sum;\n    }\n\n    public static void main(String[] args) {\n        int[] numbers = {1, 2, 3};\n        System.out.println(\"total \" + total(numbers));\n    }\n}\n",
            ),
        ],
        "src/main/java/sample/Main.java",
        "mvn -q compile exec:java -Dexec.mainClass=sample.Main",
        4,
        "numbers",
        "int[3]",
        "total 6",
    );
}

/// Go with Delve, which builds the program itself.
#[gpui_kit::test]
#[ignore]
fn debug_go_live(cx: &mut TestAppContext) {
    debug_project_live(
        cx,
        "go",
        &[
            (
                "main.go",
                "package main\n\nimport \"fmt\"\n\nfunc total(numbers []int) int {\n\tsum := 0\n\tfor _, n := range numbers {\n\t\tsum += n\n\t}\n\treturn sum\n}\n\nfunc main() {\n\tnumbers := []int{1, 2, 3}\n\tfmt.Printf(\"total %d\\n\", total(numbers))\n}\n",
            ),
            ("go.mod", "module sample\n\ngo 1.22\n"),
        ],
        "main.go",
        "go run .",
        6,
        "numbers",
        "[]int len: 3, cap: 3, [1,2,3]",
        "total 6",
    );
}
