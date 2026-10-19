//! The Map in place of the code: getting there from the code, zooming
//! between Plan and Site and back into the code, Go Back through it, and
//! the debugger on its Plan.

use gpui_kit::{Modifiers, PinchEvent, ScrollDelta, ScrollWheelEvent, TouchPhase, point};

use super::*;
use crate::map::plan::Focus;
use crate::map::view::{Level, MapView};

const GO_BACK: &str = if cfg!(target_os = "macos") {
    "ctrl--"
} else {
    "alt-left"
};

const STORE: &str = "//! Keeps what's sent.\n\npub fn save(item: u8) -> bool {\n    check(item);\n    write(item)\n}\n\nfn check(item: u8) {}\n\nfn write(item: u8) -> bool {\n    true\n}\n";

/// A small Rust project, `src/store.rs` open with the cursor in `save`.
fn open_project(
    cx: &mut TestAppContext,
) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
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
    write("web/App.svelte", "<script></script>\n");
    let (window, workspace) = open(cx, &dir.path().join("src/store.rs"));
    let inside = STORE.find("check(item);").unwrap();
    cx.update(|cx| {
        workspace
            .read(cx)
            .editor()
            .clone()
            .select(inside..inside, cx)
    });
    (dir, window, workspace)
}

fn map(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Option<Entity<MapView>> {
    cx.update(|cx| workspace.read(cx).map_showing().cloned())
}

fn level(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Option<Level> {
    let view = map(cx, workspace)?;
    Some(cx.update(|cx| view.read(cx).level().clone()))
}

/// The Map's level once the project is read.
fn read_level(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Level {
    wait_until(cx, |cx| {
        map(cx, workspace).is_some_and(|view| cx.update(|cx| view.read(cx).project().is_some()))
    });
    level(cx, workspace).unwrap()
}

fn plan_of(name: &str) -> impl Fn(&Level) -> bool + '_ {
    move |level| matches!(level, Level::Plan(Focus::Function(symbol)) if symbol.name == name)
}

fn draw(cx: &mut TestAppContext, window: AnyWindowHandle) {
    step(cx, window, |window, cx| window.render_frame(cx));
}

/// The gesture that changed level has ended, and the canvas is drawn.
fn settle(cx: &mut TestAppContext, window: AnyWindowHandle, view: &Entity<MapView>) {
    view.update(cx, |view, _| view.end_gesture());
    draw(cx, window);
}

/// ⌘-scroll on the canvas at `at`: down zooms out, up zooms in.
fn zoom_scroll(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    at: gpui_kit::Point<gpui_kit::Pixels>,
    dy: f32,
) {
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.simulate_event(ScrollWheelEvent {
        position: at,
        delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
        modifiers: Modifiers::secondary_key(),
        touch_phase: TouchPhase::Moved,
    });
    vcx.run_until_parked();
}

#[gpui_kit::test]
fn show_map_flies_out_of_the_function_at_the_cursor(cx: &mut TestAppContext) {
    let (dir, window, workspace) = open_project(cx);
    press(cx, window, "secondary-b");
    let level = read_level(cx, &workspace);
    assert!(plan_of("save")(&level), "{level:?}");
    let view = map(cx, &workspace).unwrap();
    let (titles, selected, fading) = cx.update(|cx| {
        let view = view.read(cx);
        let titles: Vec<String> = view.plan().nodes.iter().map(|n| n.title.clone()).collect();
        (titles, view.selected_keys().to_vec(), view.crossfading())
    });
    assert_eq!(titles, ["save", "check", "write", "main"]);
    assert_eq!(selected.len(), 1, "you are here: the function came from");
    assert!(selected[0].contains("save"));
    assert!(fading, "flown out, fading in");
    // Its signature on its box.
    let signature = cx.update(|cx| view.read(cx).plan().nodes[0].detail.clone());
    assert_eq!(signature, "(item: u8) → bool");
    // ⌘B again: the code.
    press(cx, window, "secondary-b");
    assert!(map(cx, &workspace).is_none());
    // The places are kept, out of git.
    assert!(dir.path().join(".jig/map/places.json").is_file());
    let ignore = std::fs::read_to_string(dir.path().join(".jig/.gitignore")).unwrap();
    assert!(ignore.lines().any(|line| line == "map/places.json"));
}

#[gpui_kit::test]
fn the_title_bar_button_shows_the_map_and_the_code(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let button = vcx
        .debug_bounds("map-toggle")
        .expect("the Map button")
        .center();
    vcx.simulate_click(button, Modifiers::none());
    read_level(cx, &workspace);
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let button = vcx.debug_bounds("map-toggle").unwrap().center();
    vcx.simulate_click(button, Modifiers::none());
    assert!(map(cx, &workspace).is_none());
}

#[gpui_kit::test]
fn keys_step_between_code_plan_and_site(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    press(cx, window, "secondary-b");
    read_level(cx, &workspace);
    let view = map(cx, &workspace).unwrap();
    // Out to Site: the file the function's in selected.
    press(cx, window, "secondary-up");
    assert_eq!(level(cx, &workspace), Some(Level::Site));
    let selected = cx.update(|cx| view.read(cx).selected_keys().to_vec());
    assert_eq!(selected, ["site:file:src/store.rs"]);
    assert!(cx.update(|cx| view.read(cx).crossfading()));
    // In again: the file's Plan.
    press(cx, window, "secondary-down");
    assert_eq!(
        level(cx, &workspace),
        Some(Level::Plan(Focus::Module("src/store.rs".into())))
    );
    // Enter on a function: its Plan; Enter on that: its code.
    let check = cx.update(|cx| view.read(cx).titled("check"));
    let key = cx.update(|cx| view.read(cx).plan().nodes[check].key.clone());
    draw(cx, window);
    let at = cx.update(|cx| view.read(cx).node_point(check));
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.simulate_click(at, Modifiers::none());
    assert_eq!(
        cx.update(|cx| view.read(cx).selected_keys().to_vec()),
        [key]
    );
    press(cx, window, "enter");
    assert!(plan_of("check")(&level(cx, &workspace).unwrap()));
    press(cx, window, "enter");
    assert!(map(cx, &workspace).is_none(), "into its code");
    let cursor = cx.update(|cx| workspace.read(cx).editor().cursor(cx));
    assert_eq!(cursor, STORE.find("check(item: u8)").unwrap());
    // Go Back retraces it all: the Plan, the file's, Site, then the code.
    press(cx, window, GO_BACK);
    assert!(plan_of("check")(&level(cx, &workspace).unwrap()));
    press(cx, window, GO_BACK);
    assert_eq!(
        level(cx, &workspace),
        Some(Level::Plan(Focus::Module("src/store.rs".into())))
    );
    press(cx, window, GO_BACK);
    assert_eq!(level(cx, &workspace), Some(Level::Site));
    press(cx, window, GO_BACK);
    assert!(plan_of("save")(&level(cx, &workspace).unwrap()));
    press(cx, window, GO_BACK);
    assert!(map(cx, &workspace).is_none(), "back in the code");
}

#[gpui_kit::test]
fn zooming_past_a_level_crosses_into_the_next(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    press(cx, window, "secondary-b");
    read_level(cx, &workspace);
    let view = map(cx, &workspace).unwrap();
    settle(cx, window, &view);
    // Within Plan, zooming only zooms.
    let middle = cx.update(|cx| view.read(cx).node_point(0));
    zoom_scroll(cx, window, middle, 60.);
    assert!(plan_of("save")(&level(cx, &workspace).unwrap()));
    assert!(cx.update(|cx| view.read(cx).zoom()) < 1.);
    // Past its far end, out to Site.
    for _ in 0..6 {
        zoom_scroll(cx, window, middle, 120.);
        if level(cx, &workspace) == Some(Level::Site) {
            break;
        }
    }
    assert_eq!(level(cx, &workspace), Some(Level::Site));
    // The gesture that crossed goes no further until it's let go.
    let zoom = cx.update(|cx| view.read(cx).zoom());
    zoom_scroll(cx, window, middle, 120.);
    assert_eq!(cx.update(|cx| view.read(cx).zoom()), zoom);
    settle(cx, window, &view);
    // Zooming in over a file: in place, then into its Plan.
    for _ in 0..30 {
        let at = cx.update(|cx| view.read(cx).site_point("file:src/store.rs"));
        zoom_scroll(cx, window, at, -150.);
        draw(cx, window);
        if level(cx, &workspace) != Some(Level::Site) {
            break;
        }
    }
    assert_eq!(
        level(cx, &workspace),
        Some(Level::Plan(Focus::Module("src/store.rs".into())))
    );
    settle(cx, window, &view);
    // Over a function, in to its Plan; over that one, in to its code.
    for target in ["save", ""] {
        for _ in 0..12 {
            let ix = cx.update(|cx| {
                let view = view.read(cx);
                match target {
                    "" => view.plan().focus.unwrap(),
                    title => view.titled(title),
                }
            });
            let at = cx.update(|cx| view.read(cx).node_point(ix));
            zoom_scroll(cx, window, at, -150.);
            if target == "save" && plan_of("save")(&level(cx, &workspace).unwrap_or(Level::Site)) {
                break;
            }
            if target.is_empty() && map(cx, &workspace).is_none() {
                break;
            }
        }
        if target == "save" {
            assert!(plan_of("save")(&level(cx, &workspace).unwrap()));
            settle(cx, window, &view);
        }
    }
    assert!(map(cx, &workspace).is_none(), "zoomed into its code");
}

#[gpui_kit::test]
fn folders_open_in_place_as_site_zooms_in(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    press(cx, window, "secondary-b");
    read_level(cx, &workspace);
    let view = map(cx, &workspace).unwrap();
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    // All the way out: the project, its folders closed.
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    let before = cx.update(|cx| view.read(cx).site_showing());
    let net = before.iter().find(|(key, _)| key == "dir:src/net/");
    let rect = cx.update(|cx| {
        let site = view.read(cx).site().unwrap();
        site.nodes[site.index_of("dir:src/net/").unwrap()].rect
    });
    // A file the index doesn't read has a place too.
    let site_keys: Vec<String> = cx.update(|cx| {
        view.read(cx)
            .site()
            .unwrap()
            .nodes
            .iter()
            .map(|n| n.key.clone())
            .collect()
    });
    assert!(site_keys.contains(&"file:web/App.svelte".to_string()));
    for _ in 0..30 {
        let at = cx.update(|cx| view.read(cx).site_point("dir:src/net/"));
        zoom_scroll(cx, window, at, -100.);
        draw(cx, window);
        let showing = cx.update(|cx| view.read(cx).site_showing());
        if showing
            .iter()
            .any(|(key, open)| key == "dir:src/net/" && *open)
        {
            break;
        }
    }
    let after = cx.update(|cx| view.read(cx).site_showing());
    assert!(
        after
            .iter()
            .any(|(key, open)| key == "dir:src/net/" && *open),
        "net opened: {after:?}, before {net:?}"
    );
    assert!(
        after.iter().any(|(key, _)| key == "file:src/net/client.rs"),
        "its file shows inside"
    );
    assert_eq!(
        level(cx, &workspace),
        Some(Level::Site),
        "never another screen"
    );
    let still = cx.update(|cx| {
        let site = view.read(cx).site().unwrap();
        site.nodes[site.index_of("dir:src/net/").unwrap()].rect
    });
    assert_eq!(rect, still, "nothing moved");
}

#[gpui_kit::test]
fn a_pinch_out_in_the_code_shows_the_map(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.simulate_mouse_move(point(px(400.), px(250.)), None, Modifiers::none());
    for _ in 0..4 {
        vcx.simulate_event(PinchEvent {
            position: point(px(400.), px(250.)),
            delta: -0.05,
            modifiers: Modifiers::none(),
            phase: TouchPhase::Moved,
        });
    }
    vcx.run_until_parked();
    let level = read_level(cx, &workspace);
    assert!(plan_of("save")(&level));
}

#[gpui_kit::test]
fn the_paused_stack_lights_up_and_running_functions_glow(cx: &mut TestAppContext) {
    let (dir, window, workspace) = open_project(cx);
    press(cx, window, "secondary-b");
    read_level(cx, &workspace);
    let view = map(cx, &workspace).unwrap();
    // Paused in `write`, called from `save`: the path lights.
    let store = super::super::tabs::canonical(&dir.path().join("src/store.rs"));
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
