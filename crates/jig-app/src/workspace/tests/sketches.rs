//! Sketching on the Map: the tools draw boxes, arrows, notes and frames,
//! placed by the nearest real box and saved as you draw; ⌘Z undoes; a
//! sketch shows a mark at other levels; ⌘K tells the agent about it;
//! Adopt makes the agent's proposal yours; a task linking the sketch is
//! its task; boxes turn solid as their code appears; Archive.

use gpui_kit::point;
use jig_ai::agent::AgentEvent;
use serde_json::json;

use super::*;
use crate::map::sketch::{Match, ShapeKind, ShapeRef};
use crate::map::view::sketch::Tool;
use crate::map::view::{Level, MapView};

const STORE: &str = "//! Keeps what's sent.\n\npub fn save(item: u8) -> bool {\n    check(item);\n    write(item)\n}\n\nfn check(item: u8) {}\n\nfn write(item: u8) -> bool {\n    true\n}\n";

/// A small Rust project, `src/store.rs` open with the cursor in `save`,
/// its Map showing at save's Plan, in a window big enough to draw in.
fn on_the_map(
    cx: &mut TestAppContext,
) -> (
    tempfile::TempDir,
    AnyWindowHandle,
    Entity<Workspace>,
    Entity<MapView>,
) {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write("Cargo.toml", "[package]\nname = \"shop\"\n");
    write(
        "src/main.rs",
        "mod store;\nuse store::save;\n\nfn main() {\n    save(1);\n}\n",
    );
    write("src/store.rs", STORE);
    write("src/net/client.rs", "pub fn send() {}\n");
    let (window, workspace) = open(cx, &dir.path().join("src/store.rs"));
    cx.simulate_window_resize(window, gpui_kit::size(px(1400.), px(900.)));
    let inside = STORE.find("check(item);").unwrap();
    cx.update(|cx| {
        workspace
            .read(cx)
            .editor()
            .clone()
            .select(inside..inside, cx)
    });
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
    view.update(cx, |view, _| view.end_gesture());
    draw(cx, window);
    (dir, window, workspace, view)
}

fn draw(cx: &mut TestAppContext, window: AnyWindowHandle) {
    step(cx, window, |window, cx| window.render_frame(cx));
}

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, at: gpui_kit::Point<gpui_kit::Pixels>) {
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.simulate_click(at, gpui_kit::Modifiers::none());
    vcx.run_until_parked();
}

fn click_on(cx: &mut TestAppContext, window: AnyWindowHandle, selector: &'static str) {
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let at = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("no {selector}"))
        .center();
    vcx.simulate_click(at, gpui_kit::Modifiers::none());
    vcx.run_until_parked();
}

/// Press at `from`, carry to `to` and let go there.
fn drag(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    from: gpui_kit::Point<gpui_kit::Pixels>,
    to: gpui_kit::Point<gpui_kit::Pixels>,
) {
    use gpui_kit::{Modifiers, MouseButton};
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.simulate_mouse_down(from, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.simulate_mouse_move(
        from + point(px(20.), px(20.)),
        MouseButton::Left,
        Modifiers::none(),
    );
    vcx.run_until_parked();
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.simulate_mouse_move(to, MouseButton::Left, Modifiers::none());
    vcx.simulate_mouse_up(to, MouseButton::Left, Modifiers::none());
    vcx.run_until_parked();
    draw(cx, window);
}

/// Type `text` into the label being edited, and press Enter.
fn label(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
    let text = text.to_string();
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        window.input(&text, cx);
    });
    press(cx, window, "enter");
}

fn sketch_file(dir: &tempfile::TempDir, name: &str) -> String {
    std::fs::read_to_string(dir.path().join(format!(".jig/map/sketches/{name}.toml")))
        .unwrap_or_default()
}

fn key(sketch: &str, kind: ShapeKind, id: &str) -> String {
    ShapeRef {
        sketch: sketch.into(),
        kind,
        id: id.into(),
    }
    .key()
}

/// Draw a box labelled `text` with the box tool, clicking somewhere clear.
fn draw_box(cx: &mut TestAppContext, window: AnyWindowHandle, view: &Entity<MapView>, text: &str) {
    press(cx, window, "b");
    assert_eq!(cx.update(|cx| view.read(cx).tool()), Tool::Box);
    let at = cx.update(|cx| view.read(cx).empty_point());
    click(cx, window, at);
    label(cx, window, text);
}

#[gpui_kit::test]
fn the_box_tool_draws_a_box_by_the_nearest_real_box_saved_and_undone(cx: &mut TestAppContext) {
    let (dir, window, _workspace, view) = on_the_map(cx);
    draw_box(cx, window, &view, "Cache");
    assert_eq!(
        cx.update(|cx| view.read(cx).tool()),
        Tool::Pointer,
        "back to the pointer"
    );
    let text = sketch_file(&dir, "untitled-sketch");
    assert!(text.contains("label = \"Cache\""), "{text}");
    assert!(text.contains("id = \"cache\""), "{text}");
    assert!(
        text.contains("level = \"plan:src/store.rs#save\""),
        "{text}"
    );
    assert!(
        text.contains("near = \"src/"),
        "placed by a real box: {text}"
    );
    // Esc picks the pointer back.
    press(cx, window, "n");
    assert_eq!(cx.update(|cx| view.read(cx).tool()), Tool::Note);
    press(cx, window, "escape");
    assert_eq!(cx.update(|cx| view.read(cx).tool()), Tool::Pointer);
    // ⌘Z takes the label back, then the box, and the file goes with it.
    press(cx, window, "secondary-z");
    assert!(sketch_file(&dir, "untitled-sketch").contains("label = \"Box\""));
    press(cx, window, "secondary-z");
    assert_eq!(sketch_file(&dir, "untitled-sketch"), "");
    press(cx, window, "secondary-shift-z");
    assert!(sketch_file(&dir, "untitled-sketch").contains("label = \"Box\""));
}

#[gpui_kit::test]
fn an_arrow_dragged_off_a_real_box_attaches_to_it_and_makes_a_box(cx: &mut TestAppContext) {
    let (dir, window, _workspace, view) = on_the_map(cx);
    let save = cx.update(|cx| {
        let view = view.read(cx);
        view.node_point(view.titled("save"))
    });
    let to = cx.update(|cx| view.read(cx).empty_point());
    press(cx, window, "a");
    drag(cx, window, save, to);
    label(cx, window, "Rate limiter");
    let sketch = cx.update(|cx| view.read(cx).saved_sketches()[0].clone());
    assert_eq!(sketch.arrows.len(), 1);
    assert_eq!(sketch.arrows[0].from, "src/store.rs#save");
    assert_eq!(sketch.arrows[0].to, "rate-limiter");
    assert_eq!(sketch.boxes[0].label, "Rate limiter");
    assert!(sketch_file(&dir, &sketch.name).contains("from = \"src/store.rs#save\""));
}

#[gpui_kit::test]
fn a_note_on_a_box_moves_with_it(cx: &mut TestAppContext) {
    let (dir, window, _workspace, view) = on_the_map(cx);
    draw_box(cx, window, &view, "Cache");
    let cache = key("untitled-sketch", ShapeKind::Box, "cache");
    let on = cx.update(|cx| view.read(cx).shape_point(&cache));
    press(cx, window, "n");
    click(cx, window, on);
    label(cx, window, "Per user");
    let sketch = cx.update(|cx| view.read(cx).saved_sketches()[0].clone());
    assert_eq!(sketch.notes[0].on, "cache");
    assert_eq!(sketch.notes[0].text, "Per user");
    let note = key("untitled-sketch", ShapeKind::Note, &sketch.notes[0].id);
    let before = cx.update(|cx| view.read(cx).shape_at(&note).unwrap());
    // The box moved: the note follows.
    drag(cx, window, on, on + point(px(0.), px(120.)));
    let after = cx.update(|cx| view.read(cx).shape_at(&note).unwrap());
    assert!(after.y > before.y + 50., "{before:?} → {after:?}");
    assert!(sketch_file(&dir, "untitled-sketch").contains("on = \"cache\""));
    // ⌫ takes the box out, with its note.
    click(cx, window, on + point(px(0.), px(120.)));
    press(cx, window, "backspace");
    assert!(cx.update(|cx| view.read(cx).saved_sketches().is_empty()));
}

#[gpui_kit::test]
fn a_sketch_shows_a_mark_at_other_levels_that_goes_there(cx: &mut TestAppContext) {
    let (_dir, window, workspace, view) = on_the_map(cx);
    draw_box(cx, window, &view, "Cache");
    let at_plan = cx.update(|cx| view.read(cx).level().clone());
    press(cx, window, "secondary-up");
    view.update(cx, |view, _| view.end_gesture());
    draw(cx, window);
    assert_eq!(read_level(cx, &workspace), Level::Site);
    assert!(cx.update(|cx| view.read(cx).sketches_showing().is_empty()));
    let marks = cx.update(|cx| view.read(cx).marks());
    assert!(
        marks
            .iter()
            .any(|(_, names)| names == &["untitled-sketch".to_string()]),
        "{marks:?}"
    );
    click_on(cx, window, "map-sketch-mark");
    assert_eq!(read_level(cx, &workspace), at_plan);
    assert_eq!(
        cx.update(|cx| view.read(cx).sketches_showing()),
        ["untitled-sketch"]
    );
}

fn read_level(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Level {
    cx.update(|cx| {
        workspace
            .read(cx)
            .map_showing()
            .unwrap()
            .read(cx)
            .level()
            .clone()
    })
}

#[gpui_kit::test]
fn command_k_on_a_sketch_tells_the_agent_about_it(cx: &mut TestAppContext) {
    let (_dir, window, workspace, view) = on_the_map(cx);
    let save = cx.update(|cx| {
        let view = view.read(cx);
        view.node_point(view.titled("save"))
    });
    let to = cx.update(|cx| view.read(cx).empty_point());
    press(cx, window, "a");
    drag(cx, window, save, to);
    label(cx, window, "Cache");
    let cache = key("untitled-sketch", ShapeKind::Box, "cache");
    let at = cx.update(|cx| view.read(cx).shape_point(&cache));
    click(cx, window, at);
    press(cx, window, "secondary-k");
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("Is this a good place for a cache?", cx);
    });
    press(cx, window, "enter");
    let prompt = cx.update(|cx| workspace.read(cx).agent_prompt()).unwrap();
    assert!(prompt.contains("My sketch “Untitled sketch”"), "{prompt}");
    assert!(prompt.contains("don't change it"), "{prompt}");
    assert!(
        prompt.contains("- box sketch:untitled-sketch/cache “Cache” (new: no code yet)"),
        "{prompt}"
    );
    assert!(prompt.contains("[selected]"), "{prompt}");
    assert!(
        prompt.contains("- arrow from src/store.rs#save to sketch:untitled-sketch/cache"),
        "{prompt}"
    );
    assert!(
        prompt.contains("Attached to:\n- src/store.rs#save"),
        "what it's attached to: {prompt}"
    );
    // The agent's note on the sketch's box lands on it.
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.test_agent_request(
                AgentEvent::Drew {
                    tool: "map_note".into(),
                    input: json!({"place": "sketch:untitled-sketch/cache", "text": "Keep it small"}),
                },
                window,
                cx,
            );
        })
    });
    let marks = cx.update(|cx| view.read(cx).lens_marks());
    assert!(
        marks
            .iter()
            .any(|(key, _, words)| key == "sketch:untitled-sketch/cache"
                && words.contains(&"Keep it small".to_string())),
        "{marks:?}"
    );
}

#[gpui_kit::test]
fn adopt_makes_the_agents_proposal_the_users_sketch(cx: &mut TestAppContext) {
    let (_dir, window, workspace, view) = on_the_map(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            this.test_agent_request(
                AgentEvent::Drew {
                    tool: "map_propose".into(),
                    input: json!({"title": "A cache", "boxes": [
                        {"name": "Cache", "near": "src/store.rs#save", "purpose": "Keeps the last item"},
                        {"name": "Evictor", "near": "src/store.rs#save"}
                    ], "arrows": [
                        {"from": "src/store.rs#save", "to": "Cache", "label": "reads"},
                        {"from": "Evictor", "to": "Cache"}
                    ]}),
                },
                window,
                cx,
            );
            this.focus_code(window, cx);
        })
    });
    draw(cx, window);
    // One ghost first.
    click_on(cx, window, "map-adopt-Cache");
    let sketch = cx.update(|cx| view.read(cx).saved_sketches()[0].clone());
    assert_eq!(sketch.title, "A cache");
    assert_eq!(sketch.boxes.len(), 1);
    assert_eq!(sketch.boxes[0].purpose, "Keeps the last item");
    assert_eq!(sketch.arrows[0].from, "src/store.rs#save");
    assert_eq!(sketch.arrows[0].label, "reads");
    // Then the rest: the proposal is settled.
    click_on(cx, window, "map-proposal-Adopt");
    let sketch = cx.update(|cx| view.read(cx).saved_sketches()[0].clone());
    assert_eq!(sketch.boxes.len(), 2);
    assert!(
        sketch
            .arrows
            .iter()
            .any(|a| a.from == "evictor" && a.to == "cache"),
        "{:?}",
        sketch.arrows
    );
    let rows = cx.update(|cx| workspace.read(cx).agent_entries());
    assert!(
        rows.iter().any(
            |row| matches!(row, ChatEntry::Drawing { settled: Some(word), .. } if word == "Adopted")
        ),
        "{rows:?}"
    );
}

/// A sketch saved by hand at save's Plan: `check` (real code), `Cache`
/// (none yet), and an arrow from save to check.
fn a_saved_sketch(dir: &tempfile::TempDir) {
    let folder = dir.path().join(".jig/map/sketches");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("caching.toml"),
        r#"title = "Caching"
level = "plan:src/store.rs#save"

[[box]]
id = "check"
label = "check"
near = "src/store.rs#save"
at = [0.0, 200.0]
size = [180.0, 64.0]

[[box]]
id = "cache"
label = "Cache"
near = "src/store.rs#save"
at = [260.0, 200.0]
size = [180.0, 64.0]

[[arrow]]
from = "src/store.rs#save"
to = "check"

[[arrow]]
from = "check"
to = "cache"
"#,
    )
    .unwrap();
}

fn read_again(cx: &mut TestAppContext, view: &Entity<MapView>, dir: &tempfile::TempDir) {
    let root = dir.path().to_path_buf();
    view.update(cx, |view, cx| {
        let before = view.project().cloned();
        let project = crate::map::index::Project::read_again(&root, before.as_deref());
        view.set_project(std::sync::Arc::new(project), cx);
    });
}

#[gpui_kit::test]
fn boxes_turn_solid_as_their_code_appears_and_then_it_archives(cx: &mut TestAppContext) {
    let (dir, window, _workspace, view) = on_the_map(cx);
    a_saved_sketch(&dir);
    read_again(cx, &view, &dir);
    assert_eq!(
        cx.update(|cx| view.read(cx).match_of("caching", "check")),
        Match::Real("src/store.rs#check".into())
    );
    assert_eq!(
        cx.update(|cx| view.read(cx).match_of("caching", "cache")),
        Match::None
    );
    assert_eq!(cx.update(|cx| view.read(cx).real_boxes("caching")), (1, 2));
    assert!(!cx.update(|cx| view.read(cx).sketch_done("caching")));
    // The code appears, and calls along the arrow.
    std::fs::write(
        dir.path().join("src/cache.rs"),
        "pub struct Cache;\n\nimpl Cache {\n    pub fn keep() {}\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/store.rs"),
        STORE.replace(
            "fn check(item: u8) {}",
            "fn check(item: u8) {\n    Cache::keep();\n}",
        ),
    )
    .unwrap();
    read_again(cx, &view, &dir);
    assert_eq!(
        cx.update(|cx| view.read(cx).match_of("caching", "cache")),
        Match::Real("src/cache.rs#Cache".into())
    );
    assert!(cx.update(|cx| view.read(cx).sketch_done("caching")));
    draw(cx, window);
    click_on(cx, window, "map-sketch-archive");
    assert!(cx.update(|cx| view.read(cx).sketches_showing().is_empty()));
    assert!(sketch_file(&dir, "caching").contains("archived = true"));
    // Still in the Sketches list, to restore.
    click_on(cx, window, "map-sketches");
    click_on(cx, window, "map-sketch-restore-0");
    assert_eq!(
        cx.update(|cx| view.read(cx).sketches_showing()),
        ["caching"]
    );
}

#[gpui_kit::test]
fn an_unsure_match_stays_sketched(cx: &mut TestAppContext) {
    let (dir, _window, _workspace, view) = on_the_map(cx);
    std::fs::write(dir.path().join("src/net/limit.rs"), "pub struct Limiter;\n").unwrap();
    std::fs::write(dir.path().join("src/limit.rs"), "pub struct Limiter;\n").unwrap();
    let folder = dir.path().join(".jig/map/sketches");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(
        folder.join("limits.toml"),
        "title = \"Limits\"\nlevel = \"site\"\n\n[[box]]\nid = \"limiter\"\nlabel = \"Limiter\"\nat = [0.0, 0.0]\nsize = [180.0, 64.0]\n",
    )
    .unwrap();
    read_again(cx, &view, &dir);
    assert_eq!(
        cx.update(|cx| view.read(cx).match_of("limits", "limiter")),
        Match::Unsure(2)
    );
}

#[gpui_kit::test]
fn make_task_asks_the_agent_and_the_task_links_back(cx: &mut TestAppContext) {
    let (dir, window, workspace, view) = on_the_map(cx);
    a_saved_sketch(&dir);
    read_again(cx, &view, &dir);
    draw(cx, window);
    click_on(cx, window, "map-sketch-make-task");
    let prompt = cx.update(|cx| workspace.read(cx).agent_prompt()).unwrap();
    assert!(
        prompt.contains("Turn my sketch “Caching” into a task"),
        "{prompt}"
    );
    assert!(prompt.contains("create_task"), "{prompt}");
    assert!(
        prompt.contains("Sketch: [Caching](.jig/map/sketches/caching.toml)"),
        "{prompt}"
    );
    assert!(
        prompt.contains("- box sketch:caching/cache"),
        "the sketch: {prompt}"
    );
    // The agent saves it, linking the sketch: the sketch's task.
    let task = jig_ai::tasks::create(
        dir.path(),
        "Add a cache",
        "Keep the last item.\n\nSketch: [Caching](.jig/map/sketches/caching.toml)",
        vec![jig_ai::tasks::Todo::new("Add `Cache`")],
    )
    .unwrap();
    view.update(cx, |view, cx| {
        view.link_tasks();
        cx.notify();
    });
    assert!(sketch_file(&dir, "caching").contains(&format!("task = \"{}\"", task.id)));
    // Implement runs that task.
    draw(cx, window);
    click_on(cx, window, "map-sketch-implement");
    let prompt = cx.update(|cx| workspace.read(cx).agent_prompt()).unwrap();
    assert!(
        prompt.contains("Implement the task “Add a cache”"),
        "{prompt}"
    );
    assert!(prompt.contains("- [ ] Add `Cache`"), "{prompt}");
    assert!(
        prompt.contains(".jig/map/sketches/caching.toml"),
        "{prompt}"
    );
}

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
    let folder = root.join(crate::map::sketch::DIR);
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
                match crate::map::lens::call(call["tool"].as_str()?, &call["input"]).ok()? {
                    crate::map::lens::Call::Draw(drawing) => Some(drawing),
                    crate::map::lens::Call::Clear => None,
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
    let folder = root.join(crate::map::sketch::DIR);
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
        crate::settings::init(cx);
        crate::providers::init(cx);
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
