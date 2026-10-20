//! The Map's Domain level: Jig's drafts under the banner, Index Domains
//! and Update Domains as conversations whose answers are proposals to
//! keep, editing by hand with ⌘Z, the tints at Site, and sharing.

use gpui_kit::{Modifiers, MouseButton, point};

use super::*;
use crate::map::view::{Level, MapView};

const SAVED: &str = r#"[[domain]]
name = "Bookings"
purpose = "Book a place at an event."
offers = ["Booking form"]
why = "They take bookings."
members = ["src/bookings/"]

[[domain]]
name = "Events"
purpose = "List what's on."
offers = ["Event list"]
why = "They show events."
members = ["src/events/", "web/"]

[[flow]]
from = "Bookings"
to = "Events"
label = "asks for events"

[shared]
members = ["src/util.rs", "src/main.rs"]
"#;

/// A small project with two business areas, a shared file and a page the
/// index can't read; `src/main.rs` open.
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
    write("Cargo.toml", "[package]\nname = \"venue\"\n");
    write(
        "src/main.rs",
        "mod bookings;\nmod events;\nmod util;\n\nfn main() {\n    bookings::book();\n}\n",
    );
    write(
        "src/bookings/mod.rs",
        "pub fn book() {\n    crate::events::find_event();\n    pay_booking();\n}\n\nfn pay_booking() {}\n",
    );
    write(
        "src/events/mod.rs",
        "pub fn find_event() {}\n\npub fn list_events() {}\n",
    );
    write("src/util.rs", "pub fn log_line() {}\n");
    write("web/App.svelte", "<script></script>\n");
    if saved {
        write(".jig/map/domains.toml", SAVED);
    }
    let (window, workspace) = open(cx, &dir.path().join("src/main.rs"));
    (dir, window, workspace)
}

fn map(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Option<Entity<MapView>> {
    cx.update(|cx| workspace.read(cx).map_showing().cloned())
}

fn draw(cx: &mut TestAppContext, window: AnyWindowHandle) {
    step(cx, window, |window, cx| window.render_frame(cx));
}

/// The Map at Domain, read and drawn, its fade over.
fn to_domain(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
) -> Entity<MapView> {
    press(cx, window, "secondary-b");
    wait_until(cx, |cx| {
        map(cx, workspace).is_some_and(|view| cx.update(|cx| view.read(cx).project().is_some()))
    });
    let view = map(cx, workspace).unwrap();
    press(cx, window, "secondary-up");
    press(cx, window, "secondary-up");
    assert_eq!(cx.update(|cx| view.read(cx).level().clone()), Level::Domain);
    view.update(cx, |view, _| view.end_gesture());
    draw(cx, window);
    view
}

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, at: gpui_kit::Point<gpui_kit::Pixels>) {
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
}

fn click_on(cx: &mut TestAppContext, window: AnyWindowHandle, selector: &'static str) {
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let at = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("no {selector}"))
        .center();
    vcx.simulate_click(at, Modifiers::none());
    vcx.run_until_parked();
}

/// Press at `from`, carry to `to` and let go there.
fn drag(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    from: gpui_kit::Point<gpui_kit::Pixels>,
    to: gpui_kit::Point<gpui_kit::Pixels>,
) {
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

fn saved(dir: &tempfile::TempDir) -> String {
    std::fs::read_to_string(dir.path().join(".jig/map/domains.toml")).unwrap_or_default()
}

/// As if the agent had finished its turn saying `reply`.
fn answer(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    reply: &str,
) {
    let ws = workspace.clone();
    let reply = reply.to_string();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.test_agent_request(jig_ai::agent::AgentEvent::Done(reply), window, cx)
        })
    });
    wait_until(cx, |cx| {
        map(cx, workspace).is_some_and(|view| {
            cx.update(|cx| view.read(cx).level() == &Level::Domain && view.read(cx).has_proposal())
        })
    });
}

#[gpui_kit::test]
fn domain_shows_jigs_drafts_under_the_index_banner(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, false);
    let view = to_domain(cx, window, &workspace);
    let (regions, banner) =
        cx.update(|cx| (view.read(cx).domain_regions(), view.read(cx).banner()));
    assert_eq!(
        banner.as_deref(),
        Some("Index Domains to name and describe these")
    );
    // The folders, by the words their functions use.
    assert!(regions.len() >= 3, "{regions:?}");
    assert!(
        regions.iter().any(|name| name.contains("booking")),
        "{regions:?}"
    );
    // Drafts can't be edited by hand.
    let event = cx.update(|cx| view.read(cx).region_point(&regions[0], false));
    click(cx, window, event);
    press(cx, window, "f2");
    assert!(cx.update(|cx| view.read(cx).domain_input()).is_none());
    // ⌘↓ goes into Site.
    press(cx, window, "secondary-down");
    assert_eq!(cx.update(|cx| view.read(cx).level().clone()), Level::Site);
}

#[gpui_kit::test]
fn index_domains_is_a_conversation_whose_answer_is_kept_on_the_map(cx: &mut TestAppContext) {
    let (dir, window, workspace) = open_project(cx, false);
    let view = to_domain(cx, window, &workspace);
    click_on(cx, window, "map-banner-index");
    let prompt = cx
        .update(|cx| workspace.read(cx).agent_prompt())
        .expect("a conversation");
    assert!(prompt.starts_with("You are indexing this project's business domains"));
    // Every source file, the page the index can't read too.
    assert!(prompt.contains("- web/App.svelte\n"), "{prompt}");
    assert!(prompt.contains("never routes"));
    // A command that only reads runs without asking; others ask.
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.test_agent_request(
                jig_ai::agent::AgentEvent::Permission(jig_ai::agent::PermissionRequest {
                    id: "per_1".into(),
                    title: "Run a command".into(),
                    detail: "rg -n book src | head".into(),
                    always: None,
                }),
                window,
                cx,
            )
        })
    });
    assert_eq!(cx.update(|cx| workspace.read(cx).agent_waiting_on()), None);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.test_agent_request(
                jig_ai::agent::AgentEvent::Permission(jig_ai::agent::PermissionRequest {
                    id: "per_2".into(),
                    title: "Run a command".into(),
                    detail: "rm -rf target".into(),
                    always: None,
                }),
                window,
                cx,
            )
        })
    });
    assert_eq!(
        cx.update(|cx| workspace.read(cx).agent_waiting_on()),
        Some("permission")
    );
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.settle_agent_request(false, window, cx));
    });
    answer(
        cx,
        window,
        &workspace,
        &format!("Here:\n\n```toml\n{SAVED}```\n"),
    );
    let view = map(cx, &workspace).unwrap_or(view);
    let (regions, banner) =
        cx.update(|cx| (view.read(cx).domain_regions(), view.read(cx).banner()));
    assert_eq!(banner.as_deref(), Some("The agent proposes these domains"));
    assert_eq!(regions, ["Bookings", "Events", "Shared"]);
    assert!(saved(&dir).is_empty(), "nothing saved before it's kept");
    // Kept: saved, and shown as the domains.
    press(cx, window, "secondary-enter");
    assert!(!cx.update(|cx| view.read(cx).has_proposal()));
    let text = saved(&dir);
    assert!(text.contains("name = \"Bookings\""), "{text}");
    assert!(
        text.contains("members = [\n  \"src/bookings/\"\n]"),
        "{text}"
    );
    assert_eq!(cx.update(|cx| view.read(cx).banner()), None);
    // ⌘Z takes back the keeping: the proposal again.
    press(cx, window, "secondary-z");
    assert!(cx.update(|cx| view.read(cx).has_proposal()));
    assert!(saved(&dir).is_empty());
}

#[gpui_kit::test]
fn domains_are_renamed_merged_and_undone_by_hand(cx: &mut TestAppContext) {
    let (dir, window, workspace) = open_project(cx, true);
    let view = to_domain(cx, window, &workspace);
    // Rename: select it, F2, type, Enter.
    let at = cx.update(|cx| view.read(cx).region_point("Bookings", false));
    click(cx, window, at);
    press(cx, window, "f2");
    assert!(cx.update(|cx| view.read(cx).domain_input()).is_some());
    press(cx, window, "secondary-a");
    type_slowly(cx, window, "Reservations");
    press(cx, window, "enter");
    let text = saved(&dir);
    assert!(text.contains("name = \"Reservations\""), "{text}");
    assert!(
        text.contains("from = \"Reservations\""),
        "the flow follows: {text}"
    );
    // Merge: Events dropped onto Reservations.
    draw(cx, window);
    let (from, to) = cx.update(|cx| {
        (
            view.read(cx).region_point("Events", false),
            view.read(cx).region_point("Reservations", true),
        )
    });
    drag(cx, window, from, to);
    let text = saved(&dir);
    assert_eq!(text.matches("[[domain]]").count(), 1, "{text}");
    assert!(text.contains("\"web/\""), "its members came along: {text}");
    // ⌘Z undoes the merge, then the rename.
    press(cx, window, "secondary-z");
    assert_eq!(saved(&dir).matches("[[domain]]").count(), 2);
    press(cx, window, "secondary-z");
    assert!(saved(&dir).contains("name = \"Bookings\""));
    press(cx, window, "secondary-shift-z");
    assert!(saved(&dir).contains("name = \"Reservations\""));
}

#[gpui_kit::test]
fn a_file_dragged_at_site_moves_to_the_domain_it_lands_on(cx: &mut TestAppContext) {
    let (dir, window, workspace) = open_project(cx, true);
    let view = to_domain(cx, window, &workspace);
    press(cx, window, "secondary-down");
    let view = map(cx, &workspace).unwrap_or(view);
    view.update(cx, |view, _| {
        view.end_gesture();
        view.show_site_box("file:src/util.rs");
    });
    draw(cx, window);
    assert_eq!(cx.update(|cx| view.read(cx).level().clone()), Level::Site);
    // Tinted by their domains, until the switch is off.
    assert_eq!(
        cx.update(|cx| view.read(cx).tint_at("src/events/mod.rs")),
        Some(1)
    );
    assert_eq!(cx.update(|cx| view.read(cx).tint_at("src/util.rs")), None);
    let (from, to) = cx.update(|cx| {
        (
            view.read(cx).site_point("file:src/util.rs"),
            view.read(cx).site_point("file:web/App.svelte"),
        )
    });
    drag(cx, window, from, to);
    let text = saved(&dir);
    assert!(
        text.contains("\"src/events/\",\n  \"web/\",\n  \"src/util.rs\""),
        "{text}"
    );
    assert!(!text.contains("members = [\n  \"src/util.rs\""));
    view.update(cx, |view, cx| view.set_tints(false, cx));
    assert_eq!(
        cx.update(|cx| view.read(cx).tint_at("src/events/mod.rs")),
        None
    );
}

#[gpui_kit::test]
fn update_domains_places_only_the_unsorted_files(cx: &mut TestAppContext) {
    let (dir, window, workspace) = open_project(cx, true);
    std::fs::create_dir_all(dir.path().join("src/gifts")).unwrap();
    std::fs::write(
        dir.path().join("src/gifts/mod.rs"),
        "pub fn give_gift() {}\n",
    )
    .unwrap();
    let view = to_domain(cx, window, &workspace);
    assert!(
        cx.update(|cx| view.read(cx).domain_regions())
            .contains(&"Unsorted".to_string())
    );
    click_on(cx, window, "map-update-domains");
    let prompt = cx.update(|cx| workspace.read(cx).agent_prompt()).unwrap();
    assert!(prompt.contains("- src/gifts/mod.rs\n"));
    assert!(
        prompt.contains("name = \"Bookings\""),
        "the saved domains come along"
    );
    answer(
        cx,
        window,
        &workspace,
        r#"```toml
[[place]]
file = "src/gifts/mod.rs"
domain = "Gift Cards"

[[place]]
file = "src/events/mod.rs"
domain = "Gift Cards"

[[new_domain]]
name = "Gift Cards"
purpose = "Give a gift."
```"#,
    );
    let view = map(cx, &workspace).unwrap_or(view);
    press(cx, window, "secondary-enter");
    let text = saved(&dir);
    assert!(text.contains("name = \"Gift Cards\""));
    assert!(
        text.contains("members = [\n  \"src/gifts/mod.rs\"\n]"),
        "{text}"
    );
    // What was confirmed stays where it was.
    assert!(
        text.contains("members = [\n  \"src/events/\",\n  \"web/\"\n]"),
        "{text}"
    );
    assert!(
        !cx.update(|cx| view.read(cx).domain_regions())
            .contains(&"Unsorted".to_string())
    );
}

#[gpui_kit::test]
fn the_domains_are_copied_as_markdown_and_drawn_as_an_image(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    let view = to_domain(cx, window, &workspace);
    click_on(cx, window, "map-copy-markdown");
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("copied");
    assert!(
        copied.contains("## Bookings\n\nBook a place at an event.\n\nOffers:\n- Booking form\n")
    );
    assert!(copied.contains("- Bookings asks for events Events\n"));
    let svg = cx.update(|cx| view.read(cx).export_svg(cx));
    assert!(svg.starts_with("<svg"));
    assert!(svg.contains(">Bookings</text>") && svg.contains("asks for events"));
}

#[gpui_kit::test]
fn names_sit_above_their_regions_and_flows_in_the_gaps(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    to_domain(cx, window, &workspace);
    draw(cx, window);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let selector = |name: String| -> &'static str { Box::leak(name.into_boxed_str()) };
    let regions: Vec<gpui_kit::Bounds<gpui_kit::Pixels>> = (0..4)
        .filter_map(|ix| vcx.debug_bounds(selector(format!("map-region-{ix}"))))
        .collect();
    assert!(regions.len() >= 2, "{regions:?}");
    for (ix, region) in regions.iter().enumerate() {
        let name = vcx
            .debug_bounds(selector(format!("map-region-name-{ix}")))
            .expect("a name");
        // Above its region, whole.
        assert!(name.bottom() <= region.top() + px(0.5), "{name:?}");
        assert!(name.size.width > px(0.));
    }
    // The flow's label covers no region.
    let flow = vcx.debug_bounds("map-flow-0").expect("a flow label");
    for region in &regions {
        assert!(!flow.intersects(region), "{flow:?} covers {region:?}");
    }
}

#[gpui_kit::test]
fn flows_are_faint_but_for_the_selected_domains(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, true);
    let view = to_domain(cx, window, &workspace);
    // main.rs is Shared's: the Bookings–Events flow isn't its, so faint.
    let selected = cx.update(|cx| view.read(cx).selected_keys().to_vec());
    assert_eq!(selected, ["domain:/shared"]);
    assert_eq!(cx.update(|cx| view.read(cx).drawn_faint()), [true]);
    // Bookings picked: its flow in full.
    let at = cx.update(|cx| view.read(cx).region_point("Bookings", true));
    click(cx, window, at);
    draw(cx, window);
    assert_eq!(cx.update(|cx| view.read(cx).drawn_faint()), [false]);
    // Nothing picked: faint again.
    press(cx, window, "escape");
    draw(cx, window);
    assert_eq!(cx.update(|cx| view.read(cx).drawn_faint()), [true]);
}
