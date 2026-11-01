//! The Map in place of the code: getting there from the code, zooming
//! between Plan and Site and back into the code, Go Back through it, and
//! the debugger on its Plan.

use gpui_kit::{Modifiers, PinchEvent, ScrollDelta, ScrollWheelEvent, TouchPhase, point};

use super::*;
use jig_map::Focus;
use jig_map::{Level, MapView};

mod levels;
mod live;
mod pictures;

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
