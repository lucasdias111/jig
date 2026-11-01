//! The debugger on the Plan: the paused stack, and functions glowing as a real program runs them.

use super::*;

#[gpui_kit::test]
fn the_paused_stack_lights_up_and_running_functions_glow(cx: &mut TestAppContext) {
    let (dir, window, workspace) = open_project(cx);
    press(cx, window, "secondary-b");
    read_level(cx, &workspace);
    let view = map(cx, &workspace).unwrap();
    // Paused in `write`, called from `save`: the path lights.
    let store = super::super::super::tabs::canonical(&dir.path().join("src/store.rs"));
    let write_line = STORE[..STORE.find("    true").unwrap()]
        .matches('\n')
        .count() as u32;
    let save_line = STORE[..STORE.find("    write(item)").unwrap()]
        .matches('\n')
        .count() as u32;
    let stack = vec![(store.clone(), write_line), (store.clone(), save_line)];
    view.update(cx, |view, cx| view.set_stack(stack, cx));
    assert_eq!(cx.update(|cx| view.read(cx).lit_path()), ["write", "save"]);
    // Running: each function on the Plan reports its first statement.
    let lines = cx.update(|cx| view.read(cx).live_lines());
    assert!(lines.contains(&(store.clone(), write_line)));
    view.update(cx, |view, cx| view.fire_line(&store, write_line, cx));
    assert!(cx.update(|cx| view.read(cx).firing_of("write")) > 0.5);
    assert_eq!(cx.update(|cx| view.read(cx).firing_of("check")), 0.);
}

/// Opens `files` in a scratch project with `run` as its one run
/// configuration (`App`, unless `run` is empty) and debugger `debugger`
/// on, then the Map of `source`'s `main`.
fn open_live(
    cx: &mut TestAppContext,
    debugger: &str,
    files: &[(&str, &str)],
    source: &str,
    run: &str,
) -> (
    tempfile::TempDir,
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<MapView>,
) {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for (name, text) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    if !run.is_empty() {
        std::fs::create_dir_all(root.join(".jig")).unwrap();
        std::fs::write(
            root.join(".jig/run.toml"),
            format!("[[run]]\nname = \"App\"\n{run}"),
        )
        .unwrap();
    }
    let (window, workspace) = open(cx, &root);
    let key = debugger.to_string();
    step(cx, window, move |_, cx| {
        jig_settings::update(cx, |s| {
            s.debugging.set_enabled(&key, true);
            // C and Swift aren't in everyone's list from the start.
            for language in ["c", "swift"] {
                if !s.languages.added.iter().any(|added| added == language) {
                    s.languages.added.push(language.into());
                }
            }
        })
    });
    let path = root.join(source);
    open_file(cx, window, &workspace, &path);
    let text = std::fs::read_to_string(&path).unwrap();
    let inside = text.find("big(item)").expect("main calls big");
    cx.update(|cx| {
        workspace
            .read(cx)
            .editor()
            .clone()
            .select(inside..inside, cx)
    });
    press(cx, window, "secondary-b");
    let level = read_level(cx, &workspace);
    assert!(plan_of("main")(&level), "{level:?}");
    let view = map(cx, &workspace).unwrap();
    (dir, window, workspace, view)
}

/// Waits, a tick at a time, for `done`, calling `each` on every tick.
fn wait_live(
    cx: &mut TestAppContext,
    workspace: &Entity<Workspace>,
    ticks: usize,
    each: &mut dyn FnMut(&mut TestAppContext),
    done: &dyn Fn(&Workspace, &gpui_kit::App) -> bool,
) {
    for _ in 0..ticks {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(20));
        cx.run_until_parked();
        each(cx);
        if cx.update(|cx| done(workspace.read(cx), cx)) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let output = cx.update(|cx| workspace.read(cx).run_output(cx));
    panic!("timed out: {output:#?}");
}

/// Debugs run configuration `name` with no breakpoint, the Map showing
/// `main`'s Plan: `big` and `small` glow as the program goes round its
/// loop, it never stops, and it prints `done` with none of the logpoints'
/// messages.
fn fires_off_live(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    view: &Entity<MapView>,
    name: &str,
) {
    use crate::workspace::debug::Phase;
    let lines = cx.update(|cx| view.read(cx).live_lines());
    assert!(lines.len() >= 3, "main, big and small report: {lines:?}");
    let mut seen = std::collections::BTreeSet::new();
    let mut watch = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            for title in ["big", "small"] {
                if view.read(cx).firing_of(title) > 0. {
                    seen.insert(title);
                }
            }
        })
    };
    press(cx, window, "ctrl-d");
    let wanted = name.to_string();
    wait_live(cx, workspace, 1500, &mut watch, &move |this, cx| {
        this.run_picker_rows(cx)
            .is_some_and(|rows| rows.contains(&wanted))
    });
    let typed = name.to_string();
    step(cx, window, move |window, cx| window.input(&typed, cx));
    press(cx, window, "enter");
    // Building and starting a debugger can take a while.
    wait_live(cx, workspace, 4000, &mut watch, &|this, _| {
        assert_ne!(this.debug_phase(), Some(Phase::Paused), "it never stops");
        this.debug_phase() == Some(Phase::Ended)
    });
    let (_, lines, _) = cx.update(|cx| workspace.read(cx).run_output(cx)).unwrap();
    let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
    eprintln!("{texts:#?}");
    assert!(texts.iter().any(|text| text == "done"), "it ran to the end");
    assert!(
        !texts.iter().any(|text| text.contains("jig-fired")),
        "the logpoints' messages stay out of the console"
    );
    assert!(
        !texts
            .iter()
            .any(|text| text.starts_with("(lldb)") || text == "True"),
        "nor does setting up LLDB"
    );
    assert_eq!(
        seen,
        ["big", "small"].into(),
        "both ways the `if` went glowed"
    );
}

const CARGO: &str = "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n";

/// With the real LLDB and no breakpoint, the functions on the Plan glow
/// as the program runs through them, and it carries on. Needs `lldb-dap`
/// (Xcode's, or one on the PATH); macOS may ask once to allow debugging.
#[gpui_kit::test]
#[ignore]
fn fires_off_as_it_runs_live(cx: &mut TestAppContext) {
    let (_dir, window, workspace, view) = open_live(
        cx,
        "rust",
        &[
            ("Cargo.toml", CARGO),
            (
                "src/main.rs",
                "fn main() {\n    for item in 0..60 {\n        if item % 2 == 0 {\n            big(item);\n        } else {\n            small(item);\n        }\n        std::thread::sleep(std::time::Duration::from_millis(50));\n    }\n    println!(\"done\");\n}\n\nfn big(item: i32) {\n    std::hint::black_box(item);\n}\n\nfn small(item: i32) {\n    std::hint::black_box(item);\n}\n",
            ),
        ],
        "src/main.rs",
        "",
    );
    fires_off_live(cx, window, &workspace, &view, "sample");
}

/// Paused at a breakpoint in `big` with the real LLDB, the Map stays on
/// `main`'s Plan with `big` and `main` lit as the stack; stepping out
/// leaves `main` lit, and running on to the end, nothing. Needs what
/// `fires_off_as_it_runs_live` needs.
#[gpui_kit::test]
#[ignore]
fn stepping_lights_the_way_live(cx: &mut TestAppContext) {
    use crate::workspace::debug::Phase;
    let source = "fn main() {\n    let items = vec![1, 5];\n    for item in items {\n        if item > 2 {\n            big(item);\n        } else {\n            small(item);\n        }\n    }\n    done();\n}\n\nfn big(item: i32) {\n    std::hint::black_box(item);\n}\n\nfn small(item: i32) {\n    std::hint::black_box(item);\n}\n\nfn done() {\n    println!(\"done\");\n}\n";
    let (_dir, window, workspace, view) = open_live(
        cx,
        "rust",
        &[("Cargo.toml", CARGO), ("src/main.rs", source)],
        "src/main.rs",
        "",
    );
    // A breakpoint in `big`, set in the code, then back to `main`'s Plan.
    press(cx, window, "secondary-b");
    let inside = source.find("    std::hint::black_box").unwrap();
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        ws.update(cx, |this, cx| this.editor().select(inside..inside, cx));
        window.press("secondary-f8", cx);
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert_eq!(this.breakpoint_lines(this.active, cx), [13].into());
    });
    let in_main = source.find("big(item);").unwrap();
    cx.update(|cx| {
        workspace
            .read(cx)
            .editor()
            .clone()
            .select(in_main..in_main, cx)
    });
    press(cx, window, "secondary-b");
    assert!(plan_of("main")(&read_level(cx, &workspace)));
    let mut idle = |_: &mut TestAppContext| {};
    press(cx, window, "ctrl-d");
    wait_live(cx, &workspace, 1500, &mut idle, &|this, cx| {
        this.run_picker_rows(cx)
            .is_some_and(|rows| rows.iter().any(|row| row == "sample"))
    });
    step(cx, window, |window, cx| window.input("sample", cx));
    press(cx, window, "enter");
    wait_live(cx, &workspace, 4000, &mut idle, &|this, _| {
        this.debug_phase() == Some(Phase::Paused)
    });
    // The stack comes a moment after the pause.
    wait_live(cx, &workspace, 500, &mut idle, &|_, cx| {
        !view.read(cx).lit_path().is_empty()
    });
    assert!(map(cx, &workspace).is_some(), "the Map stays while paused");
    assert!(plan_of("main")(&level(cx, &workspace).unwrap()));
    assert_eq!(cx.update(|cx| view.read(cx).lit_path()), ["big", "main"]);

    // Out of `big`, back in `main`.
    press(cx, window, "shift-f8");
    wait_live(cx, &workspace, 1500, &mut idle, &|this, _| {
        this.debug_phase() == Some(Phase::Paused)
    });
    wait_live(cx, &workspace, 500, &mut idle, &|_, cx| {
        view.read(cx).lit_path() == ["main"]
    });

    // On to the end: nothing's paused.
    press(cx, window, "f9");
    wait_live(cx, &workspace, 1500, &mut idle, &|this, _| {
        this.debug_phase() == Some(Phase::Ended)
    });
    assert!(cx.update(|cx| view.read(cx).lit_path()).is_empty());
}

fn needs_debuggers_dir() {
    assert!(
        std::env::var_os("JIG_DEBUGGERS_DIR").is_some(),
        "JIG_DEBUGGERS_DIR: where the debugger is installed"
    );
}

/// `files` debugged with `debugger` as run configuration `App`.
fn fires_off_in(
    cx: &mut TestAppContext,
    debugger: &str,
    files: &[(&str, &str)],
    source: &str,
    run: &str,
) {
    let (_dir, window, workspace, view) = open_live(cx, debugger, files, source, run);
    fires_off_live(cx, window, &workspace, &view, "App");
}

/// JavaScript with js-debug. Needs Node, and `JIG_DEBUGGERS_DIR`.
#[gpui_kit::test]
#[ignore]
fn fires_off_in_javascript_live(cx: &mut TestAppContext) {
    needs_debuggers_dir();
    fires_off_in(
        cx,
        "typescript",
        &[(
            "main.js",
            "const seen = [];\n\nfunction main() {\n  for (let item = 0; item < 60; item++) {\n    if (item % 2 === 0) {\n      big(item);\n    } else {\n      small(item);\n    }\n    pause(50);\n  }\n  console.log('done');\n}\n\nfunction big(item) {\n  seen.push(item);\n}\n\nfunction small(item) {\n  seen.push(item);\n}\n\nfunction pause(ms) {\n  Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms);\n}\n\nmain();\n",
        )],
        "main.js",
        "command = \"node main.js\"\n",
    );
}

/// Python with debugpy. Needs `JIG_DEBUGGERS_DIR`.
#[gpui_kit::test]
#[ignore]
fn fires_off_in_python_live(cx: &mut TestAppContext) {
    needs_debuggers_dir();
    fires_off_in(
        cx,
        "python",
        &[(
            "main.py",
            "import time\n\nseen = []\n\n\ndef main():\n    for item in range(60):\n        if item % 2 == 0:\n            big(item)\n        else:\n            small(item)\n        time.sleep(0.05)\n    print(\"done\")\n\n\ndef big(item):\n    seen.append(item)\n\n\ndef small(item):\n    seen.append(item)\n\n\nmain()\n",
        )],
        "main.py",
        "command = \"python3 main.py\"\n",
    );
}

/// Go with Delve. Needs Go, and `JIG_DEBUGGERS_DIR`.
#[gpui_kit::test]
#[ignore]
fn fires_off_in_go_live(cx: &mut TestAppContext) {
    needs_debuggers_dir();
    fires_off_in(
        cx,
        "go",
        &[
            (
                "main.go",
                "package main\n\nimport (\n\t\"fmt\"\n\t\"time\"\n)\n\nvar seen []int\n\nfunc main() {\n\tfor item := 0; item < 60; item++ {\n\t\tif item%2 == 0 {\n\t\t\tbig(item)\n\t\t} else {\n\t\t\tsmall(item)\n\t\t}\n\t\ttime.Sleep(50 * time.Millisecond)\n\t}\n\tfmt.Println(\"done\")\n}\n\nfunc big(item int) {\n\tseen = append(seen, item)\n}\n\nfunc small(item int) {\n\tseen = append(seen, item)\n}\n",
            ),
            ("go.mod", "module sample\n\ngo 1.22\n"),
        ],
        "main.go",
        "command = \"go run .\"\n",
    );
}

/// C with LLDB, built with the system's compiler.
#[gpui_kit::test]
#[ignore]
fn fires_off_in_c_live(cx: &mut TestAppContext) {
    fires_off_in(
        cx,
        "rust",
        &[(
            "main.c",
            "#include <stdio.h>\n#include <unistd.h>\n\nstatic volatile int seen;\n\nvoid big(int item);\nvoid small(int item);\n\nint main(void) {\n    for (int item = 0; item < 60; item++) {\n        if (item % 2 == 0) {\n            big(item);\n        } else {\n            small(item);\n        }\n        usleep(50000);\n    }\n    printf(\"done\\n\");\n    return 0;\n}\n\nvoid big(int item) {\n    seen += item;\n}\n\nvoid small(int item) {\n    seen += item;\n}\n",
        )],
        "main.c",
        "command = \"./app\"\nprogram = \"app\"\nbuild = \"cc -g -O0 main.c -o app\"\n",
    );
}

/// Swift with LLDB. Needs Xcode's tools.
#[gpui_kit::test]
#[ignore]
fn fires_off_in_swift_live(cx: &mut TestAppContext) {
    fires_off_in(
        cx,
        "rust",
        &[(
            "main.swift",
            "import Foundation\n\nvar seen = 0\n\nfunc main() {\n    for item in 0..<60 {\n        if item % 2 == 0 {\n            big(item)\n        } else {\n            small(item)\n        }\n        usleep(50000)\n    }\n    print(\"done\")\n}\n\nfunc big(_ item: Int) {\n    seen += item\n}\n\nfunc small(_ item: Int) {\n    seen += item\n}\n\nmain()\n",
        )],
        "main.swift",
        "command = \"./app\"\nprogram = \"app\"\nbuild = \"swiftc -g -Onone main.swift -o app\"\n",
    );
}
