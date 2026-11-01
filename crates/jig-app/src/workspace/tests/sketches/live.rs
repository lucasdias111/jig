//! Sketches with real projects and a real agent.

use super::*;

/// Pictures of a sketch on a real project's Map: `JIG_MAP_ROOT=<project>
/// JIG_MAP_FILE=<file in it> JIG_SKETCH=<sketch .toml> JIG_PICTURES=<folder>
/// cargo test -p jig-app sketch_pictures_live -- --ignored --nocapture`,
/// with `JIG_MAP_DRAWING` (a JSON list of `{tool, input}`) for the agent's
/// answer beside it. The sketch is put in the project's sketches folder
/// while the pictures are made, then taken out again.
#[gpui_kit::test]
#[ignore = "draws a real project"]
fn sketch_pictures_live(cx: &mut TestAppContext) {
    let root = std::path::PathBuf::from(std::env::var("JIG_MAP_ROOT").unwrap());
    let file = std::path::PathBuf::from(std::env::var("JIG_MAP_FILE").unwrap());
    let sketch = std::path::PathBuf::from(std::env::var("JIG_SKETCH").unwrap());
    let out = std::path::PathBuf::from(std::env::var("JIG_PICTURES").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let name = sketch.file_stem().unwrap().to_string_lossy().into_owned();
    let folder = root.join(jig_map::SKETCH_DIR);
    let had_folder = folder.exists();
    std::fs::create_dir_all(&folder).unwrap();
    let placed = folder.join(format!("{name}.toml"));
    std::fs::copy(&sketch, &placed).unwrap();
    let (window, workspace) = open(cx, &root);
    open_file(cx, window, &workspace, &file);
    cx.simulate_window_resize(window, gpui_kit::size(px(1600.), px(1000.)));
    press(cx, window, "secondary-b");
    wait_until(cx, |cx| {
        cx.update(|cx| {
            workspace
                .read(cx)
                .map_showing()
                .is_some_and(|view| view.read(cx).project().is_some())
        })
    });
    let view = cx.update(|cx| workspace.read(cx).map_showing().cloned().unwrap());
    let go = name.clone();
    view.update(cx, |view, cx| {
        view.go_to_sketch(&go, cx);
        view.end_gesture();
    });
    draw(cx, window);
    let write = |what: &str, bytes: Vec<u8>| {
        let path = out.join(format!("{name}-{what}.png"));
        std::fs::write(&path, bytes).unwrap();
        println!("wrote {}", path.display());
    };
    if let Some(bytes) = view.update(cx, |view, cx| view.picture_now(cx)) {
        write("sketch", bytes);
    }
    let boxes: Vec<(String, Match)> = cx.update(|cx| {
        let view = view.read(cx);
        view.saved_sketches()
            .iter()
            .filter(|s| s.name == name)
            .flat_map(|s| {
                s.boxes
                    .iter()
                    .map(|b| (b.label.clone(), view.match_of(&s.name, &b.id)))
            })
            .collect()
    });
    println!("boxes: {boxes:?}");
    if let Some(drawing) = std::env::var_os("JIG_MAP_DRAWING") {
        let calls: Vec<serde_json::Value> =
            serde_json::from_str(&std::fs::read_to_string(drawing).unwrap()).unwrap();
        let drawings = calls
            .iter()
            .filter_map(|call| {
                match jig_map::drawing_call(call["tool"].as_str()?, &call["input"]).ok()? {
                    jig_map::Call::Draw(drawing) => Some(drawing),
                    jig_map::Call::Clear => None,
                }
            })
            .enumerate()
            .collect();
        view.update(cx, |view, cx| view.set_drawings(drawings, cx));
        draw(cx, window);
        if let Some(bytes) = view.update(cx, |view, cx| view.picture_now(cx)) {
            write("answer", bytes);
        }
    }
    // A level up: the mark that goes back to it.
    press(cx, window, "secondary-up");
    view.update(cx, |view, _| view.end_gesture());
    draw(cx, window);
    println!("marks: {:?}", cx.update(|cx| view.read(cx).marks()));
    if let Some(bytes) = view.update(cx, |view, cx| view.picture_now(cx)) {
        write("level-up", bytes);
    }
    std::fs::remove_file(&placed).unwrap();
    if !had_folder {
        let _ = std::fs::remove_dir(&folder);
    }
}

/// Wait for the agent's turn to end, up to `seconds`, allowing what only
/// reads and turning down anything else it asks to do.
fn wait_for_turn(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    seconds: u64,
) {
    let started = std::time::Instant::now();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        let waiting = cx.update(|cx| {
            let this = workspace.read(cx);
            this.agent_waiting_on()
                .map(|what| (what, this.waiting_on_a_read()))
        });
        if let Some((what, reads)) = waiting {
            println!("the agent asks: {what}, only reading: {reads}");
            let ws = workspace.clone();
            step(cx, window, move |window, cx| {
                ws.update(cx, |this, cx| {
                    this.settle_agent_request(reads, window, cx);
                })
            });
        }
        let done = cx.update(|cx| {
            workspace
                .read(cx)
                .chat
                .as_ref()
                .is_none_or(|run| !run.bubble.is_running())
        });
        if done || started.elapsed().as_secs() > seconds {
            println!("turn took {:?}", started.elapsed());
            return;
        }
    }
}

/// Asks a real agent about a sketch on a real project, then has it make a
/// task of it: `JIG_MAP_ROOT=<project> JIG_MAP_FILE=<file in it>
/// JIG_SKETCH=<sketch .toml> JIG_MAP_ASK=<question> JIG_PICTURES=<folder>
/// cargo test -p jig-app sketch_agent_live -- --ignored --nocapture`, after
/// `cargo build -p jig-app`. The sketch and the task it makes are taken
/// out of the project after.
#[gpui_kit::test]
#[ignore = "needs OpenCode, a provider and network access"]
fn sketch_agent_live(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = std::path::PathBuf::from(std::env::var("JIG_MAP_ROOT").unwrap());
    let file = std::path::PathBuf::from(std::env::var("JIG_MAP_FILE").unwrap());
    let sketch = std::path::PathBuf::from(std::env::var("JIG_SKETCH").unwrap());
    let ask = std::env::var("JIG_MAP_ASK").unwrap();
    let out = std::path::PathBuf::from(std::env::var("JIG_PICTURES").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let jig = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/Jig");
    jig_ai::tasks::mcp::set_command(jig.canonicalize().unwrap());
    let name = sketch.file_stem().unwrap().to_string_lossy().into_owned();
    let folder = root.join(jig_map::SKETCH_DIR);
    let had_folder = folder.exists();
    std::fs::create_dir_all(&folder).unwrap();
    let placed = folder.join(format!("{name}.toml"));
    std::fs::copy(&sketch, &placed).unwrap();
    let tasks_before: Vec<String> = jig_ai::tasks::list(&root)
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.id)
        .collect();
    let (window, workspace) = open(cx, &root);
    cx.update(|cx| {
        jig_settings::init(cx);
        jig_agent::providers::init(cx);
    });
    open_file(cx, window, &workspace, &file);
    cx.simulate_window_resize(window, gpui_kit::size(px(1600.), px(1000.)));
    press(cx, window, "secondary-b");
    wait_until(cx, |cx| {
        cx.update(|cx| {
            workspace
                .read(cx)
                .map_showing()
                .is_some_and(|view| view.read(cx).project().is_some())
        })
    });
    let view = cx.update(|cx| workspace.read(cx).map_showing().cloned().unwrap());
    let go = name.clone();
    view.update(cx, |view, cx| {
        view.go_to_sketch(&go, cx);
        view.end_gesture();
        view.select_sketch(&go);
    });
    draw(cx, window);
    // Asking about it.
    press(cx, window, "secondary-k");
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        window.input(&ask, cx);
    });
    press(cx, window, "enter");
    wait_for_turn(cx, window, &workspace, 1500);
    let entries = cx.update(|cx| workspace.read(cx).agent_entries());
    for entry in &entries {
        println!("{entry:?}");
    }
    draw(cx, window);
    if let Some(bytes) = view.update(cx, |view, cx| view.picture_now(cx)) {
        std::fs::write(out.join(format!("{name}-live-answer.png")), bytes).unwrap();
    }
    // Make Task, then agreeing to it.
    let make = name.clone();
    view.update(cx, |view, cx| view.make_task(&make, cx));
    cx.run_until_parked();
    wait_for_turn(cx, window, &workspace, 1500);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.send_to_agent("Yes, save it as a task.".into(), window, cx)
        })
    });
    wait_for_turn(cx, window, &workspace, 1500);
    let entries = cx.update(|cx| workspace.read(cx).agent_entries());
    for entry in entries.iter().rev().take(8) {
        println!("{entry:?}");
    }
    view.update(cx, |view, cx| {
        view.link_tasks();
        cx.notify();
    });
    let linked = cx.update(|cx| {
        view.read(cx)
            .saved_sketches()
            .iter()
            .find(|s| s.name == name)
            .and_then(|s| s.task.clone())
    });
    println!("linked task: {linked:?}");
    if let Some(id) = &linked {
        println!(
            "{}",
            std::fs::read_to_string(root.join(format!(".jig/tasks/{id}.md"))).unwrap_or_default()
        );
        std::fs::copy(
            root.join(format!(".jig/tasks/{id}.md")),
            out.join(format!("{name}-task.md")),
        )
        .ok();
    }
    draw(cx, window);
    if let Some(bytes) = view.update(cx, |view, cx| view.picture_now(cx)) {
        std::fs::write(out.join(format!("{name}-with-task.png")), bytes).unwrap();
    }
    // The project as it was.
    for task in jig_ai::tasks::list(&root).unwrap_or_default() {
        if !tasks_before.contains(&task.id) {
            let _ = std::fs::remove_file(root.join(format!(".jig/tasks/{}.md", task.id)));
        }
    }
    let _ = std::fs::remove_file(&placed);
    if !had_folder {
        let _ = std::fs::remove_dir(&folder);
    }
    assert!(linked.is_some(), "the task links back to the sketch");
}
