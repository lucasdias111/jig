//! The Map in place of the code: getting there from the code, zooming
//! between Plan and Site and back into the code, Go Back through it, and
//! the debugger on its Plan.

use gpui_kit::{Modifiers, PinchEvent, ScrollDelta, ScrollWheelEvent, TouchPhase, point};

use super::*;
use jig_map::Focus;
use jig_map::{Level, MapView};

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
fn the_title_bar_switch_shows_the_map_and_the_code(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let button = vcx
        .debug_bounds("mode-map")
        .expect("the Map's segment")
        .center();
    vcx.simulate_click(button, Modifiers::none());
    read_level(cx, &workspace);
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let button = vcx.debug_bounds("mode-code").unwrap().center();
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
fn command_plus_and_minus_zoom_the_map_in_steps(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    press(cx, window, "secondary-b");
    read_level(cx, &workspace);
    let view = map(cx, &workspace).unwrap();
    settle(cx, window, &view);
    let zoom = |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).zoom());
    let start = zoom(cx);
    press(cx, window, "secondary-=");
    assert!(zoom(cx) > start * 1.2, "a step closer");
    assert!(plan_of("save")(&level(cx, &workspace).unwrap()));
    press(cx, window, "secondary--");
    assert!((zoom(cx) - start).abs() < 0.01, "and back");
    press(cx, window, "secondary--");
    assert!(zoom(cx) < start, "a step further out");
    press(cx, window, "secondary-0");
    assert!((zoom(cx) - start).abs() < 0.01, "the Plan as it starts");
    // ⌘+ as well, where + has its own key.
    press(cx, window, "secondary-+");
    assert!(zoom(cx) > start * 1.2, "⌘+ is closer too");
}

#[gpui_kit::test]
fn the_ruler_shows_how_far_out_and_goes_to_a_level(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    press(cx, window, "secondary-b");
    read_level(cx, &workspace);
    let view = map(cx, &workspace).unwrap();
    settle(cx, window, &view);
    let depth = |cx: &mut TestAppContext| cx.update(|cx| view.read(cx).depth());
    let start = depth(cx);
    assert!((0.5..1.5).contains(&start), "at Plan: {start}");
    // Zooming out moves the needle towards Site, before it gets there.
    press(cx, window, "secondary--");
    let out = depth(cx);
    assert!(out > start && out < 2., "{start} then {out}");
    assert!(plan_of("save")(&level(cx, &workspace).unwrap()));
    // Clicking a level's name goes there.
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let site = vcx
        .debug_bounds("map-level-Site")
        .expect("the ruler's Site")
        .center();
    vcx.simulate_click(site, Modifiers::none());
    assert_eq!(level(cx, &workspace), Some(Level::Site));
    assert!((1.5..=2.5).contains(&depth(cx)));
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let code = vcx.debug_bounds("map-level-Code").unwrap().center();
    vcx.simulate_click(code, Modifiers::none());
    assert!(matches!(level(cx, &workspace), Some(Level::Plan(_))));
    // At Site with nothing picked, Code goes back to where you were.
    press(cx, window, "secondary-up");
    assert_eq!(level(cx, &workspace), Some(Level::Site));
    press(cx, window, "escape");
    assert!(cx.update(|cx| view.read(cx).selected_keys().is_empty()));
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let code = vcx.debug_bounds("map-level-Code").unwrap().center();
    vcx.simulate_click(code, Modifiers::none());
    assert!(map(cx, &workspace).is_none(), "back in the code");
    let cursor = cx.update(|cx| workspace.read(cx).editor().cursor(cx));
    assert_eq!(cursor, STORE.find("check(item);").unwrap());
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
fn a_pinch_in_the_code_shows_the_ruler_on_its_way_out(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx);
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    assert!(
        vcx.debug_bounds("map-ruler").is_none(),
        "not while just coding"
    );
    let at = point(px(400.), px(250.));
    vcx.simulate_mouse_move(at, None, Modifiers::none());
    let pinch = |phase, delta| PinchEvent {
        position: at,
        delta,
        modifiers: Modifiers::none(),
        phase,
    };
    vcx.simulate_event(pinch(TouchPhase::Moved, -0.05));
    vcx.run_until_parked();
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    assert!(vcx.debug_bounds("map-ruler").is_some(), "on its way out");
    assert!(map(cx, &workspace).is_none(), "not there yet");
    // Let go short of the Map: the ruler goes.
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.simulate_event(pinch(TouchPhase::Ended, 0.));
    vcx.run_until_parked();
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    assert!(vcx.debug_bounds("map-ruler").is_none());
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

/// The Map on Jig's own repo, drawn at every level: `cargo test -p
/// jig-app map_of_jig_live -- --ignored --nocapture`.
#[gpui_kit::test]
#[ignore]
fn map_of_jig_live(cx: &mut TestAppContext) {
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let repo = super::super::tabs::canonical(&repo);
    let file = repo.join("crates/jig-app/src/workspace.rs");
    let text = std::fs::read_to_string(&file).unwrap();
    let (window, workspace) = open(cx, &file);
    let inside = text.find("fn render_code").unwrap() + 3;
    cx.update(|cx| {
        workspace
            .read(cx)
            .editor()
            .clone()
            .select(inside..inside, cx)
    });
    press(cx, window, "secondary-b");
    let arrived = read_level(cx, &workspace);
    assert!(plan_of("render_code")(&arrived), "{arrived:?}");
    let view = map(cx, &workspace).unwrap();
    draw(cx, window);
    let titles: Vec<String> = cx.update(|cx| {
        view.read(cx)
            .plan()
            .nodes
            .iter()
            .map(|n| n.title.clone())
            .collect()
    });
    println!("Plan of render_code: {titles:?}");
    assert!(
        titles.iter().any(|t| t == "Workspace.render"),
        "called from render"
    );
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    let selected = cx.update(|cx| view.read(cx).selected_keys().to_vec());
    // Its type's box, or its file's when that's the type's.
    assert!(
        selected.len() == 1 && selected[0].contains(":crates/jig-app/src/workspace.rs"),
        "{selected:?}"
    );
    let showing = cx.update(|cx| view.read(cx).site_showing());
    println!("Site around workspace.rs: {} boxes showing", showing.len());
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    println!(
        "The domains: {:?}",
        cx.update(|cx| view.read(cx).domain_regions())
    );
    let middle = point(px(400.), px(240.));
    for _ in 0..12 {
        zoom_scroll(cx, window, middle, -120.);
        draw(cx, window);
        if level(cx, &workspace) != Some(Level::Site) {
            break;
        }
    }
    let ended = level(cx, &workspace);
    println!("Zoomed in from the middle: {ended:?}");
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
        crate::settings::update(cx, |s| {
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
    let output = cx.update(|cx| workspace.read(cx).run_output());
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
    let (_, lines, _) = cx.update(|cx| workspace.read(cx).run_output()).unwrap();
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

/// Pictures of a project's Map, to look at the design without a screen,
/// written to `JIG_PICTURES`: its Plan, Site and Domain at stepped zooms,
/// then a zoom from Domain into the region holding the function, on
/// through Site to its Plan, in a window 1600 by 1000. `JIG_MAP_ROOT=<project> JIG_MAP_FILE=<file>
/// JIG_MAP_AT=<text in a function> JIG_PICTURES=<dir> cargo test -p jig-app
/// map_pictures_live -- --ignored --nocapture`.
#[gpui_kit::test]
#[ignore]
fn map_pictures_live(cx: &mut TestAppContext) {
    let file = std::path::PathBuf::from(std::env::var("JIG_MAP_FILE").unwrap());
    let at = std::env::var("JIG_MAP_AT").unwrap();
    let out = std::path::PathBuf::from(std::env::var("JIG_PICTURES").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let text = std::fs::read_to_string(&file).unwrap();
    let (window, workspace) = match std::env::var_os("JIG_MAP_ROOT") {
        Some(root) => {
            let (window, workspace) = open(cx, std::path::Path::new(&root));
            open_file(cx, window, &workspace, &file);
            (window, workspace)
        }
        None => open(cx, &file),
    };
    // A laptop's window, not the tests' small one.
    cx.simulate_window_resize(window, gpui_kit::size(px(1600.), px(1000.)));
    draw(cx, window);
    let inside = text.find(&at).unwrap() + 1;
    cx.update(|cx| {
        workspace
            .read(cx)
            .editor()
            .clone()
            .select(inside..inside, cx)
    });
    press(cx, window, "secondary-b");
    println!("arrived at {:?}", read_level(cx, &workspace));
    let view = map(cx, &workspace).unwrap();
    // The agent's drawings over every level, from `JIG_MAP_DRAWING`: a
    // JSON list of `{"tool": "map_trace", "input": {…}}`.
    if let Some(file) = std::env::var_os("JIG_MAP_DRAWING") {
        let calls: Vec<serde_json::Value> =
            serde_json::from_str(&std::fs::read_to_string(file).unwrap()).unwrap();
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
        // `JIG_MAP_TOUR=<step>`: touring the first drawing, there.
        if let Some(step) = std::env::var("JIG_MAP_TOUR")
            .ok()
            .and_then(|step| step.parse().ok())
        {
            view.update(cx, |view, _| view.show_tour_step(0, step));
        }
    }
    draw(cx, window);
    let write = |name: String, bytes: Vec<u8>| {
        let path = out.join(format!("{name}.png"));
        std::fs::write(&path, bytes).unwrap();
        println!("wrote {}", path.display());
    };
    let save = |level: &str, view: &Entity<MapView>, cx: &mut TestAppContext| {
        for (name, bytes) in view.update(cx, |view, cx| view.pictures(cx)) {
            write(format!("{level}-{name}"), bytes);
        }
    };
    save("1-plan", &view, cx);
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    let selected = cx.update(|cx| view.read(cx).selected_keys().to_vec());
    println!("site selected: {selected:?}");
    let target = selected[0].trim_start_matches("site:").to_string();
    save("2-site", &view, cx);
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    save("3-domain", &view, cx);
    // The whole of Domain, then zooming over the function's file: Domain
    // and Site share their drawing, so it's where it'll be at Site.
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    if let Some(bytes) = view.update(cx, |view, cx| view.picture_now(cx)) {
        write("3-domain-canvas".into(), bytes);
    }
    for n in 0..60 {
        let at = cx.update(|cx| view.read(cx).site_point(&target));
        zoom_scroll(cx, window, at, -40.);
        view.update(cx, |view, _| view.end_gesture());
        draw(cx, window);
        if n % 3 == 2 {
            let now = level(cx, &workspace);
            println!("step {n}: {now:?}");
            if let Some(bytes) = view.update(cx, |view, cx| view.picture_now(cx)) {
                write(format!("4-zoom-{:02}", n / 3), bytes);
            }
            if !matches!(now, Some(Level::Domain) | Some(Level::Site)) {
                break;
            }
        }
    }
}
