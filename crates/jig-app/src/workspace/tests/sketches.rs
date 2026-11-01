//! Sketching on the Map: the tools draw boxes, arrows, notes and frames,
//! placed by the nearest real box and saved as you draw; ⌘Z undoes; a
//! sketch shows a mark at other levels; ⌘K tells the agent about it;
//! Adopt makes the agent's proposal yours; a task linking the sketch is
//! its task; boxes turn solid as their code appears; Archive.

use gpui_kit::point;
use jig_ai::agent::AgentEvent;
use serde_json::json;

use super::*;
use jig_map::SketchTool as Tool;
use jig_map::{Level, MapView};
use jig_map::{ShapeKind, ShapeRef, SketchMatch as Match};

mod drawing;
mod live;
mod matching;

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
