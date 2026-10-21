//! The Map's Site level, made from the domains: each region opening onto
//! its share of each component as Domain zooms on into it, coming out of a
//! Plan onto the folder holding the function, calls between regions ending
//! at their edges, tests and Shared's calls when asked, Jig's drafts
//! standing in before Index Domains, and the Plan of a file the index
//! can't read.

use gpui_kit::{Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase, point};

use super::*;
use crate::map::plan::Focus;
use crate::map::site::Show;
use crate::map::view::{Level, MapView};

const SAVED: &str = r#"[[domain]]
name = "Bookings"
purpose = "Book a place at an event."
members = ["api/src/bookings/", "api/tests/", "web/src/bookings/"]

[[domain]]
name = "Events"
purpose = "List what's on."
members = ["api/src/events/", "web/src/events/"]

[[domain]]
name = "Payments"
purpose = "Take the money."
members = ["api/src/pay/"]

[[flow]]
from = "Bookings"
to = "Events"
label = "asks for events"

[[flow]]
from = "Bookings"
to = "Payments"
label = "asks to be paid"

[shared]
members = ["api/src/util.rs"]
"#;

const BOOK: &str = "pub fn book() {\n    find_event();\n    charge();\n}\n";

/// A project of two components, an API and a web client, each with code
/// for bookings and events; `api/src/bookings/book.rs` open in `book`.
fn open_project(
    cx: &mut TestAppContext,
    saved: bool,
) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let write = |name: &str, text: &str| {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    };
    write(".git/HEAD", "ref: refs/heads/main\n");
    write("api/Cargo.toml", "[package]\nname = \"api\"\n");
    write("api/src/bookings/book.rs", BOOK);
    write(
        "api/src/bookings/cancel.rs",
        "pub fn cancel() {\n    book();\n    log_line();\n}\n",
    );
    write("api/src/events/list.rs", "pub fn find_event() {}\n");
    write(
        "api/src/events/show.rs",
        "pub fn show_event() {\n    find_event();\n}\n",
    );
    write("api/src/pay/mod.rs", "pub fn charge() {}\n");
    write("api/src/util.rs", "pub fn log_line() {}\n");
    write("api/tests/book_test.rs", "fn books() {\n    book();\n}\n");
    write("web/package.json", "{}\n");
    write(
        "web/src/bookings/form.ts",
        "export function submitBooking() {}\n",
    );
    write(
        "web/src/bookings/Form.svelte",
        "<script>\n  import { submitBooking } from './form';\n</script>\n<form></form>\n",
    );
    write(
        "web/src/bookings/page.svelte",
        "<script>\n  import Form from './Form.svelte';\n</script>\n<Form />\n",
    );
    write(
        "web/src/events/page.ts",
        "export function showEvents() {}\n",
    );
    if saved {
        write(".jig/map/domains.toml", SAVED);
    }
    let (window, workspace) = open(cx, &dir.path().join("api/src/bookings/book.rs"));
    let inside = BOOK.find("find_event").unwrap();
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

fn level(cx: &mut TestAppContext, view: &Entity<MapView>) -> Level {
    cx.update(|cx| view.read(cx).level().clone())
}

fn draw(cx: &mut TestAppContext, window: AnyWindowHandle) {
    step(cx, window, |window, cx| window.render_frame(cx));
}

fn settle(cx: &mut TestAppContext, window: AnyWindowHandle, view: &Entity<MapView>) {
    view.update(cx, |view, _| view.end_gesture());
    draw(cx, window);
}

/// The Map at the Plan of `book`, read and drawn.
fn to_plan(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
) -> Entity<MapView> {
    press(cx, window, "secondary-b");
    wait_until(cx, |cx| {
        map(cx, workspace).is_some_and(|view| cx.update(|cx| view.read(cx).project().is_some()))
    });
    let view = map(cx, workspace).unwrap();
    settle(cx, window, &view);
    view
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

fn titles_in(view: &Entity<MapView>, cx: &mut TestAppContext, key: &str) -> Vec<String> {
    cx.update(|cx| {
        let site = view.read(cx).site().unwrap();
        let ix = site.index_of(key).unwrap_or_else(|| panic!("no {key}"));
        site.nodes[ix]
            .children
            .iter()
            .map(|&c| site.nodes[c].title.clone())
            .collect()
    })
}

#[gpui_kit::test]
fn each_domain_opens_onto_its_share_of_each_component(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    let view = to_plan(cx, window, &workspace);
    let regions: Vec<String> = cx.update(|cx| {
        let site = view.read(cx).site().unwrap();
        site.regions()
            .map(|ix| site.nodes[ix].title.clone())
            .collect()
    });
    assert_eq!(regions, ["Bookings", "Events", "Payments", "Shared"]);
    // The API and the web client in both, each with its own slice, the
    // folders that only lead there folded away.
    assert_eq!(
        titles_in(&view, cx, "domain:Bookings"),
        ["api › bookings", "web › bookings"]
    );
    assert_eq!(
        titles_in(&view, cx, "domain:Events"),
        ["api › events", "web › events"]
    );
    let api_bookings = cx.update(|cx| {
        let site = view.read(cx).site().unwrap();
        let region = site.index_of("domain:Bookings").unwrap();
        site.nodes[site.nodes[region].children[0]].key.clone()
    });
    assert_eq!(
        titles_in(&view, cx, &api_bookings),
        ["book.rs", "cancel.rs"]
    );
    // Domain draws the same regions, in the same places.
    let same = cx.update(|cx| {
        let view = view.read(cx);
        let site = view.site().unwrap();
        site.regions()
            .all(|ix| view.domain_rect(&site.nodes[ix].key) == Some(site.nodes[ix].rect))
    });
    assert!(same, "Domain and Site share their drawing");
}

#[gpui_kit::test]
fn out_of_a_plan_lands_on_the_folder_holding_the_function(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    let view = to_plan(cx, window, &workspace);
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    assert_eq!(level(cx, &view), Level::Site);
    let selected = cx.update(|cx| view.read(cx).selected_keys().to_vec());
    assert_eq!(selected, ["site:file:api/src/bookings/book.rs"]);
    // Its folder open, its neighbour in view beside it.
    let showing = cx.update(|cx| view.read(cx).site_showing());
    assert!(
        showing
            .iter()
            .any(|(key, open)| key == "file:api/src/bookings/cancel.rs" && !open),
        "{showing:?}"
    );
    let (width, height) = (1000., 600.);
    for key in [
        "file:api/src/bookings/book.rs",
        "file:api/src/bookings/cancel.rs",
    ] {
        let rect = cx
            .update(|cx| view.read(cx).site_box_on_screen(key))
            .unwrap();
        assert!(
            rect.x >= 0. && rect.right() <= width && rect.y >= 0. && rect.bottom() <= height,
            "{key} in view: {rect:?}"
        );
    }
    // The region holding it is open; the others show what they're for.
    let regions = cx.update(|cx| view.read(cx).drawn_regions());
    let open = |key: &str| regions.iter().find(|(k, _, _)| k == key).map(|r| r.2);
    assert_eq!(open("domain:Bookings"), Some(1.));
}

#[gpui_kit::test]
fn domain_and_site_are_one_zoom(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    let view = to_plan(cx, window, &workspace);
    press(cx, window, "secondary-up");
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    assert_eq!(level(cx, &view), Level::Domain);
    // Only what each domain is for: no files.
    assert!(cx.update(|cx| view.read(cx).site_showing().is_empty()));
    // Zooming into Bookings carries on into Site, nothing moving.
    let mut crossed = None;
    for _ in 0..40 {
        let at = cx.update(|cx| view.read(cx).region_point("Bookings", true));
        let before = cx.update(|cx| view.read(cx).site_box_on_screen("domain:Bookings"));
        zoom_scroll(cx, window, at, -60.);
        draw(cx, window);
        if level(cx, &view) == Level::Site {
            crossed = before;
            break;
        }
    }
    assert_eq!(level(cx, &view), Level::Site, "zoomed on into Site");
    let after = cx
        .update(|cx| view.read(cx).site_box_on_screen("domain:Bookings"))
        .unwrap();
    // The same gesture carries on zooming: Domain and Site are one zoom,
    // with no letting go in between.
    let crossed_at = cx.update(|cx| view.read(cx).zoom());
    let at = cx.update(|cx| view.read(cx).region_point("Bookings", true));
    zoom_scroll(cx, window, at, -60.);
    assert!(
        cx.update(|cx| view.read(cx).zoom()) > crossed_at,
        "still zooming in"
    );
    settle(cx, window, &view);
    let before = crossed.unwrap();
    assert!(
        (after.x - before.x).abs() < before.width * 0.2
            && (after.width / before.width - 1.).abs() < 0.3,
        "the region stays where it was: {before:?} then {after:?}"
    );
    let regions = cx.update(|cx| view.read(cx).drawn_regions());
    assert!(
        regions
            .iter()
            .any(|(key, _, open)| key == "domain:Bookings" && *open > 0.),
        "opening: {regions:?}"
    );
    // And out again, Bookings picked.
    for _ in 0..40 {
        let at = cx.update(|cx| view.read(cx).region_point("Bookings", true));
        zoom_scroll(cx, window, at, 60.);
        draw(cx, window);
        if level(cx, &view) == Level::Domain {
            break;
        }
    }
    assert_eq!(level(cx, &view), Level::Domain);
    let selected = cx.update(|cx| view.read(cx).selected_keys().to_vec());
    assert_eq!(selected, ["domain:Bookings"]);
}

#[gpui_kit::test]
fn between_domains_only_their_links_worded_for_the_one_selected(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    let view = to_plan(cx, window, &workspace);
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    // book.rs is selected, in Bookings: its two links say what flows.
    let mut labels = cx.update(|cx| view.read(cx).drawn_labels());
    labels.sort();
    assert_eq!(labels, ["asks for events", "asks to be paid"]);
    // No call counts between domains, whatever is open.
    assert!(!labels.iter().any(|label| label.contains("call")));
    assert_eq!(cx.update(|cx| view.read(cx).drawn_links()), 2);
    // Bookings is lit, and its links are headed by the domain at their
    // other end.
    assert_eq!(
        cx.update(|cx| view.read(cx).lit_regions()),
        ["domain:Bookings"]
    );
    let mut ends = cx.update(|cx| view.read(cx).picked_links());
    ends.sort();
    assert_eq!(ends, ["Events", "Payments"]);
    // Nothing selected: the links stay, without words, and nothing's lit.
    press(cx, window, "escape");
    draw(cx, window);
    assert!(cx.update(|cx| view.read(cx).drawn_labels()).is_empty());
    assert!(cx.update(|cx| view.read(cx).lit_regions()).is_empty());
    assert_eq!(cx.update(|cx| view.read(cx).drawn_links()), 2);
}

#[gpui_kit::test]
fn tests_show_when_asked(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    let view = to_plan(cx, window, &workspace);
    let has_test = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            view.read(cx)
                .site()
                .unwrap()
                .index_of("file:api/tests/book_test.rs")
                .is_some()
        })
    };
    assert!(!has_test(cx));
    view.update(cx, |view, cx| view.set_site_show(Show { tests: true }, cx));
    assert!(has_test(cx));
    let _ = window;
}

#[gpui_kit::test]
fn drafts_stand_in_before_indexing(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, false);
    let view = to_plan(cx, window, &workspace);
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    // Site's regions are Jig's own groups, as Domain draws them.
    let (site, domain) = cx.update(|cx| {
        let view = view.read(cx);
        let site = view.site().unwrap();
        let regions: Vec<String> = site
            .regions()
            .map(|ix| site.nodes[ix].title.clone())
            .collect();
        (regions, view.domain_regions())
    });
    assert!(!site.is_empty());
    assert_eq!(site, domain);
    // The function's file is in one of them, open.
    let selected = cx.update(|cx| view.read(cx).selected_keys().to_vec());
    assert_eq!(selected, ["site:file:api/src/bookings/book.rs"]);
}

#[gpui_kit::test]
fn a_file_the_index_cannot_read_has_a_plan_of_what_links_to_it(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    let view = to_plan(cx, window, &workspace);
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    // Into the Svelte component from Site: its Plan, not its code.
    view.update(cx, |view, cx| {
        let ix = view
            .site()
            .unwrap()
            .index_of("file:web/src/bookings/Form.svelte")
            .unwrap();
        view.enter_site_box(ix, cx)
    });
    settle(cx, window, &view);
    assert_eq!(
        level(cx, &view),
        Level::Plan(Focus::Module("web/src/bookings/Form.svelte".into()))
    );
    let (boxes, wires, focus) = cx.update(|cx| {
        let plan = view.read(cx).plan();
        let boxes: Vec<(String, String, String, i32)> = plan
            .nodes
            .iter()
            .map(|n| (n.title.clone(), n.detail.clone(), n.note.clone(), n.column))
            .collect();
        (boxes, plan.wires.len(), plan.focus)
    });
    // The file as one box, what it is, and that its functions aren't read.
    let (title, detail, note, _) = &boxes[0];
    assert_eq!(focus, Some(0));
    assert_eq!(title, "Form.svelte");
    assert!(detail.starts_with("Svelte component · 4 lines"), "{detail}");
    assert_eq!(note, "Its functions can't be read yet");
    // The page importing it on the left, what it imports on the right.
    assert!(
        boxes.contains(&("page.svelte".into(), "imports it".into(), String::new(), -1)),
        "{boxes:?}"
    );
    assert!(
        boxes.contains(&("form.ts".into(), "imported by it".into(), String::new(), 1)),
        "{boxes:?}"
    );
    assert_eq!(wires, 2);
    // Enter on it: its code.
    press(cx, window, "enter");
    assert!(map(cx, &workspace).is_none(), "into its code");
    let title = cx.update(|cx| workspace.read(cx).document().title());
    assert_eq!(title, "Form.svelte");
}

#[gpui_kit::test]
fn a_selected_box_lights_the_arrows_carrying_its_calls(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    let view = to_plan(cx, window, &workspace);
    press(cx, window, "secondary-up");
    settle(cx, window, &view);
    // book.rs is selected: cancel.rs calls it, so that arrow is lit.
    let arrows = cx.update(|cx| view.read(cx).site_arrows_lit());
    let call = (
        "file:api/src/bookings/cancel.rs".to_string(),
        "file:api/src/bookings/book.rs".to_string(),
    );
    assert!(arrows.contains(&(call, Some(true))), "{arrows:?}");
    assert_eq!(
        arrows.iter().filter(|(_, lit)| *lit == Some(true)).count(),
        1,
        "only its own: {arrows:?}"
    );
    // Nothing selected: none lit, none faded.
    press(cx, window, "escape");
    draw(cx, window);
    let arrows = cx.update(|cx| view.read(cx).site_arrows_lit());
    assert!(arrows.iter().all(|(_, lit)| lit.is_none()), "{arrows:?}");
}
