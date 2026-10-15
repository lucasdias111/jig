//! The blueprint view in place of a Rust file's code.

use super::*;
use crate::blueprint::view::{BlueprintEvent, BlueprintView};

const GO_BACK: &str = if cfg!(target_os = "macos") {
    "ctrl--"
} else {
    "alt-left"
};

const GO_FORWARD: &str = if cfg!(target_os = "macos") {
    "ctrl-shift--"
} else {
    "alt-right"
};

const SOURCE: &str = "fn first() {\n    one();\n}\n\nfn second(ready: bool) {\n    if ready {\n        go();\n    }\n    done();\n}\n";

fn showing(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> bool {
    cx.update(|cx| workspace.read(cx).blueprint().is_some())
}

fn toggle(cx: &mut TestAppContext, window: AnyWindowHandle) {
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-b", cx);
    });
    step(cx, window, |window, cx| window.render_frame(cx));
}

fn open_rust(cx: &mut TestAppContext) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, SOURCE).unwrap();
    let (window, workspace) = open(cx, &path);
    (dir, window, workspace)
}

#[gpui_kit::test]
fn shows_the_function_at_the_cursor(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_rust(cx);
    let inside = SOURCE.find("done").unwrap();
    cx.update(|cx| {
        workspace
            .read(cx)
            .editor()
            .clone()
            .select(inside..inside, cx)
    });
    toggle(cx, window);
    assert!(showing(cx, &workspace));
    let (name, titles, functions) = cx.update(|cx| {
        let view = workspace.read(cx).blueprint().unwrap().read(cx);
        (
            view.current_function().unwrap().name.clone(),
            view.graph()
                .nodes
                .iter()
                .map(|node| node.title.clone())
                .collect::<Vec<_>>(),
            view.functions().len(),
        )
    });
    assert_eq!(name, "second");
    assert_eq!(titles, ["second", "Branch", "Call go", "Call done"]);
    assert_eq!(functions, 2);

    // The same keys go back to the code.
    toggle(cx, window);
    assert!(!showing(cx, &workspace));
}

#[gpui_kit::test]
fn clicking_a_block_selects_its_code(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_rust(cx);
    toggle(cx, window);
    // The first function: its entry, then `one();`.
    step(cx, window, |_, cx| {
        let view = workspace.read(cx).blueprint().unwrap().clone();
        view.update(cx, |view, cx| view.open_node(1, cx));
    });
    assert!(!showing(cx, &workspace));
    let selected = cx.update(|cx| workspace.read(cx).editor().selection(cx));
    assert_eq!(&SOURCE[selected], "one();");
    let focused = cx
        .update_window(window, |_, window, cx| {
            workspace
                .read(cx)
                .editor()
                .state()
                .focus_handle(cx)
                .is_focused(window)
        })
        .unwrap();
    assert!(focused, "the code has the keys");
}

#[gpui_kit::test]
fn escape_goes_back_to_the_code(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_rust(cx);
    toggle(cx, window);
    assert!(showing(cx, &workspace));
    step(cx, window, |window, cx| window.press("escape", cx));
    assert!(!showing(cx, &workspace));
}

#[gpui_kit::test]
fn picking_another_function_redraws(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_rust(cx);
    toggle(cx, window);
    step(cx, window, |_, cx| {
        let view = workspace.read(cx).blueprint().unwrap().clone();
        view.update(cx, |view, cx| view.show(1, cx));
    });
    let name = cx.update(|cx| {
        let view = workspace.read(cx).blueprint().unwrap().read(cx);
        view.current_function().unwrap().name.clone()
    });
    assert_eq!(name, "second");
}

#[gpui_kit::test]
fn only_rust_has_a_blueprint(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "fn a() {}\n").unwrap();
    let (window, workspace) = open(cx, &path);
    toggle(cx, window);
    assert!(!showing(cx, &workspace));
}

#[gpui_kit::test]
fn data_wires_can_be_hidden(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_rust(cx);
    let inside = SOURCE.find("done").unwrap();
    cx.update(|cx| {
        workspace
            .read(cx)
            .editor()
            .clone()
            .select(inside..inside, cx)
    });
    toggle(cx, window);
    let height = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let view = workspace.read(cx).blueprint().unwrap().read(cx);
            // `ready` wires the entry, which gets a row for it, to the branch.
            assert_eq!(view.graph().data.len(), 1);
            (view.layout().data, view.layout().rects[0].height)
        })
    };
    let (shown, tall) = height(cx);
    assert!(shown);
    step(cx, window, |_, cx| {
        let view = workspace.read(cx).blueprint().unwrap().clone();
        view.update(cx, |view, cx| view.set_data_wires(false, cx));
    });
    let (shown, short) = height(cx);
    assert!(!shown);
    assert!(short < tall);
}

const CALLS: &str = "fn save() {\n    write();\n}\n\nfn on_click() {\n    log();\n    save();\n}\n\nfn on_quit() {\n    save();\n}\n";

fn view(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Entity<BlueprintView> {
    cx.update(|cx| workspace.read(cx).blueprint().unwrap().clone())
}

/// The function shown, and the title of the block outlined.
fn shown(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> (String, Option<String>) {
    let view = view(cx, workspace);
    cx.update(|cx| {
        let view = view.read(cx);
        (
            view.current_function().unwrap().name.clone(),
            view.highlight()
                .map(|ix| view.graph().nodes[ix].title.clone()),
        )
    })
}

/// `CALLS`, its blueprint showing `save`.
fn open_save(cx: &mut TestAppContext) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, CALLS).unwrap();
    let (window, workspace) = open(cx, &path);
    toggle(cx, window);
    (dir, window, workspace)
}

#[gpui_kit::test]
fn lists_callers_in_the_file_without_a_server(cx: &mut TestAppContext) {
    let (_dir, _window, workspace) = open_save(cx);
    let view = view(cx, &workspace);
    let callers: Vec<_> = cx.update(|cx| {
        view.read(cx)
            .callers()
            .unwrap()
            .iter()
            .map(|caller| (caller.name.clone(), caller.place.clone()))
            .collect()
    });
    assert_eq!(
        callers,
        [
            ("on_click".to_string(), "lib.rs:7".to_string()),
            ("on_quit".to_string(), "lib.rs:11".to_string())
        ]
    );
}

#[gpui_kit::test]
fn a_caller_opens_at_the_call_and_go_back_returns(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_save(cx);
    let view = view(cx, &workspace);
    step(cx, window, |_, cx| {
        let caller = view.read(cx).callers().unwrap()[0].clone();
        view.update(cx, |_, cx| cx.emit(BlueprintEvent::OpenCaller(caller)));
    });
    assert_eq!(
        shown(cx, &workspace),
        ("on_click".into(), Some("Call save".into()))
    );
    // Callers of the caller are asked for in turn.
    let view = self::view(cx, &workspace);
    assert_eq!(
        cx.update(|cx| view.read(cx).callers().map(<[_]>::len)),
        Some(0)
    );

    step(cx, window, |window, cx| window.press(GO_BACK, cx));
    assert!(showing(cx, &workspace), "back in the blueprint");
    assert_eq!(shown(cx, &workspace).0, "save");
    step(cx, window, |window, cx| window.press(GO_FORWARD, cx));
    assert_eq!(shown(cx, &workspace).0, "on_click");
}

#[gpui_kit::test]
fn a_call_opens_what_it_calls(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_save(cx);
    let view = view(cx, &workspace);
    step(cx, window, |_, cx| {
        view.update(cx, |view, cx| view.show(1, cx))
    });
    let call = cx.update(|cx| {
        let graph = view.read(cx).graph();
        graph
            .nodes
            .iter()
            .position(|node| node.title == "Call save")
            .unwrap()
    });
    step(cx, window, |_, cx| {
        view.update(cx, |view, cx| view.step_into(call, cx))
    });
    assert_eq!(shown(cx, &workspace).0, "save");
    assert!(showing(cx, &workspace));
    step(cx, window, |window, cx| window.press(GO_BACK, cx));
    assert_eq!(shown(cx, &workspace).0, "on_click");
}

#[gpui_kit::test]
fn go_back_from_the_code_returns_to_the_blueprint(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_save(cx);
    let view = view(cx, &workspace);
    step(cx, window, |_, cx| {
        view.update(cx, |view, cx| view.open_node(1, cx))
    });
    assert!(!showing(cx, &workspace));
    step(cx, window, |window, cx| window.press(GO_BACK, cx));
    assert!(showing(cx, &workspace));
    assert_eq!(shown(cx, &workspace).0, "save");
}

/// Callers from another file, from the real rust-analyzer. Needs it
/// installed and `JIG_LIVE_LSP=1`.
#[gpui_kit::test]
#[ignore]
fn callers_from_rust_analyzer_live(cx: &mut TestAppContext) {
    assert!(
        std::env::var_os("JIG_LIVE_LSP").is_some(),
        "JIG_LIVE_LSP=1: the callers come from rust-analyzer"
    );
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    let files = [
        (
            "Cargo.toml",
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        ),
        (
            "src/lib.rs",
            "mod util;\n\npub fn run() {\n    util::helper();\n}\n",
        ),
        ("src/util.rs", "pub fn helper() {}\n"),
    ];
    for (name, text) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let (window, workspace) = open(cx, &dir.path().join("src/util.rs"));
    toggle(cx, window);
    let view = view(cx, &workspace);
    let mut callers = Vec::new();
    for _ in 0..600 {
        std::thread::sleep(std::time::Duration::from_millis(100));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        callers = cx.update(|cx| {
            view.read(cx)
                .callers()
                .map(|callers| {
                    callers
                        .iter()
                        .map(|caller| (caller.name.clone(), caller.place.clone()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        });
        if !callers.is_empty() {
            break;
        }
    }
    assert_eq!(callers, [("run".to_string(), "lib.rs:4".to_string())]);
}

/// A mouse's side button, pressed over the window.
fn mouse_navigate(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    direction: gpui_kit::NavigationDirection,
) {
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        window.dispatch_event(
            gpui_kit::PlatformInput::MouseDown(gpui_kit::MouseDownEvent {
                button: gpui_kit::MouseButton::Navigate(direction),
                position: gpui_kit::point(px(400.), px(240.)),
                click_count: 1,
                ..Default::default()
            }),
            cx,
        );
    });
}

#[gpui_kit::test]
fn mouse_back_and_forward_buttons_step_through_blueprints(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_save(cx);
    let view = view(cx, &workspace);
    // Into the code from a block, then back to the blueprint.
    step(cx, window, |_, cx| {
        view.update(cx, |view, cx| view.open_node(1, cx))
    });
    assert!(!showing(cx, &workspace));
    mouse_navigate(cx, window, gpui_kit::NavigationDirection::Back);
    assert!(
        showing(cx, &workspace),
        "the back button returns to the blueprint"
    );
    assert_eq!(shown(cx, &workspace).0, "save");
    mouse_navigate(cx, window, gpui_kit::NavigationDirection::Forward);
    assert!(!showing(cx, &workspace), "forward goes to the code again");
}

/// Click the title bar's Blueprint/Code button.
fn click_toggle(cx: &mut TestAppContext, window: AnyWindowHandle) {
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let button = vcx
        .debug_bounds("blueprint-toggle")
        .expect("the button is in the title bar");
    vcx.simulate_click(button.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
}

#[gpui_kit::test]
fn the_title_bar_button_toggles_the_blueprint(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_rust(cx);
    click_toggle(cx, window);
    assert!(showing(cx, &workspace));
    click_toggle(cx, window);
    assert!(!showing(cx, &workspace));
}

#[gpui_kit::test]
fn the_title_bar_button_is_only_for_rust(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "fn a() {}\n").unwrap();
    let (window, _workspace) = open(cx, &path);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("blueprint-toggle").is_none());
}

const SECTIONS: &str = "fn main() {}\n\nimpl App {\n    fn run(&self) {}\n}\n\n#[cfg(test)]\nmod tests {\n    fn checks() {}\n}\n";

#[gpui_kit::test]
fn test_modules_start_folded_and_open_when_shown(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    std::fs::write(&path, SECTIONS).unwrap();
    let (window, workspace) = open(cx, &path);
    toggle(cx, window);
    let view = view(cx, &workspace);
    let folded = |cx: &mut TestAppContext, section: Option<&str>| {
        cx.update(|cx| view.read(cx).folded(section))
    };
    assert!(folded(cx, Some("mod tests")));
    assert!(!folded(cx, Some("App")));
    assert!(!folded(cx, None));

    step(cx, window, |_, cx| {
        view.update(cx, |view, cx| view.toggle_section(Some("App".into()), cx))
    });
    assert!(folded(cx, Some("App")));

    // Showing a function in a folded section opens it.
    let checks = cx.update(|cx| {
        view.read(cx)
            .functions()
            .iter()
            .position(|function| function.short == "checks")
            .unwrap()
    });
    step(cx, window, |_, cx| {
        view.update(cx, |view, cx| view.show(checks, cx))
    });
    assert!(!folded(cx, Some("mod tests")));
    step(cx, window, |window, cx| window.render_frame(cx));
}

#[gpui_kit::test]
fn debugging_follows_along_in_the_blueprint(cx: &mut TestAppContext) {
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
    super::debug::fake_adapter(source.clone(), sent.clone());
    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &source);
    toggle(cx, window);
    let view = view(cx, &workspace);
    let node = |cx: &mut TestAppContext, title: &str| {
        cx.update(|cx| {
            view.read(cx)
                .graph()
                .nodes
                .iter()
                .position(|node| node.title == title)
                .unwrap()
        })
    };
    let total = node(cx, "Let total");

    // A block's dot sets a breakpoint on its line.
    step(cx, window, |_, cx| {
        view.update(cx, |view, cx| view.toggle_breakpoint(total, cx))
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert_eq!(this.breakpoint_lines(this.active, cx), [2].into());
    });
    let marks = cx.update(|cx| view.read(cx).on_blocks());
    assert!(marks.breakpoints.contains(&total));

    step(cx, window, |window, cx| window.press("ctrl-d", cx));
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).run_picker_rows(cx))
            .is_some_and(|rows| rows.first().is_some_and(|row| row == "App"))
    });
    step(cx, window, |window, cx| window.press("enter", cx));
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).debug_variable_rows().len() >= 3)
    });
    assert!(showing(cx, &workspace), "the blueprint stays while paused");
    let marks = cx.update(|cx| view.read(cx).on_blocks());
    assert_eq!(marks.paused, Some(total));
    assert_eq!(marks.visits[total], 1);

    // Resumed and finished: no pause, but the way it went stays.
    step(cx, window, |window, cx| window.press("f9", cx));
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).run_output())
            .is_some_and(|(_, _, ending)| ending.is_some())
    });
    let marks = cx.update(|cx| view.read(cx).on_blocks());
    assert_eq!(marks.paused, None);
    assert_eq!(marks.visits[total], 1);
    assert!(marks.traced());
}

/// Stepping through a loop with the real LLDB lights both ways the `if`
/// went. Needs `lldb-dap` (Xcode's, or one on the PATH); macOS may ask
/// once to allow debugging.
#[gpui_kit::test]
#[ignore]
fn stepping_lights_the_way_live(cx: &mut TestAppContext) {
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
    std::fs::write(
        &source,
        "fn main() {\n    let items = vec![1, 5];\n    for item in items {\n        if item > 2 {\n            big(item);\n        } else {\n            small(item);\n        }\n    }\n    done();\n}\n\nfn big(_: i32) {}\nfn small(_: i32) {}\nfn done() {}\n",
    )
    .unwrap();
    let (window, workspace) = open(cx, &root);
    open_file(cx, window, &workspace, &source);
    toggle(cx, window);
    let view = view(cx, &workspace);
    let node = |cx: &mut TestAppContext, title: &str| {
        cx.update(|cx| {
            view.read(cx)
                .graph()
                .nodes
                .iter()
                .position(|node| node.title == title)
                .unwrap()
        })
    };
    let first = node(cx, "Let items");
    step(cx, window, |_, cx| {
        view.update(cx, |view, cx| view.toggle_breakpoint(first, cx))
    });
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
    use crate::workspace::debug::Phase;
    step(cx, window, |window, cx| window.press("ctrl-d", cx));
    wait(cx, &|this, cx| {
        this.run_picker_rows(cx)
            .is_some_and(|rows| rows.iter().any(|row| row == "sample"))
    });
    step(cx, window, |window, cx| window.input("sample", cx));
    step(cx, window, |window, cx| window.press("enter", cx));
    wait(cx, &|this, _| this.debug_phase() == Some(Phase::Paused));
    assert!(showing(cx, &workspace));
    // Step over until `done` is next, or it ends.
    let done = node(cx, "Call done");
    for _ in 0..30 {
        if cx.update(|cx| view.read(cx).on_blocks().paused) == Some(done) {
            break;
        }
        step(cx, window, |window, cx| window.press("f8", cx));
        wait(cx, &|this, _| {
            matches!(this.debug_phase(), Some(Phase::Paused) | Some(Phase::Ended))
        });
        if cx.update(|cx| workspace.read(cx).debug_phase()) == Some(Phase::Ended) {
            break;
        }
    }
    let marks = cx.update(|cx| view.read(cx).on_blocks());
    let graph = cx.update(|cx| view.read(cx).graph().clone());
    let titles = |edges: &std::collections::HashSet<usize>| {
        let mut taken: Vec<(String, String)> = edges
            .iter()
            .map(|&edge| {
                let edge = graph.edges[edge];
                (
                    graph.nodes[edge.from].title.clone(),
                    graph.nodes[edge.to].title.clone(),
                )
            })
            .collect();
        taken.sort();
        taken
    };
    eprintln!(
        "visits {:?}\ntaken {:?}",
        marks.visits,
        titles(&marks.taken)
    );
    assert!(
        marks.visits[node(cx, "Branch")] >= 2,
        "round the loop twice"
    );
    let taken = titles(&marks.taken);
    assert!(taken.contains(&("Branch".into(), "Call big".into())));
    assert!(taken.contains(&("Branch".into(), "Call small".into())));
    step(cx, window, |window, cx| window.press("secondary-f2", cx));
}
