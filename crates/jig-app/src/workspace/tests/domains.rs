//! The Map's Domain level: Jig's drafts under the banner, Index Domains
//! and Update Domains as conversations whose answers are proposals to
//! keep, editing by hand with ⌘Z, the tints at Site, and sharing.

use gpui_kit::{Modifiers, MouseButton, point};

use super::*;
use jig_map::{Level, MapView};

mod editing;
mod indexing;

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

/// Index Domains run by the user's own agent on a real project, counting
/// what it asks the user (each turned down, so it carries on): it should
/// ask nothing and end in a proposal, which isn't kept.
/// `JIG_MAP_ROOT=<project> cargo test -p jig-app index_domains_live --
/// --ignored --nocapture`, after `cargo build`.
#[gpui_kit::test]
#[ignore]
fn index_domains_live(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = std::path::PathBuf::from(std::env::var("JIG_MAP_ROOT").unwrap());
    let jig = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/Jig");
    jig_ai::tasks::mcp::set_command(jig.canonicalize().unwrap());
    let (window, workspace) = open(cx, &root);
    cx.update(|cx| {
        jig_settings::init(cx);
        jig_agent::providers::init(cx);
    });
    press(cx, window, "secondary-b");
    wait_until(cx, |cx| {
        map(cx, &workspace).is_some_and(|view| cx.update(|cx| view.read(cx).project().is_some()))
    });
    let view = map(cx, &workspace).unwrap();
    view.update(cx, |view, cx| view.start_action(jig_map::Action::Index, cx));
    let started = std::time::Instant::now();
    let mut asked = 0;
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        if let Some(what) = cx.update(|cx| workspace.read(cx).agent_waiting_on()) {
            asked += 1;
            println!("asked for a {what}");
            let ws = workspace.clone();
            step(cx, window, move |window, cx| {
                ws.update(cx, |this, cx| this.settle_agent_request(false, window, cx));
            });
        }
        let done = cx.update(|cx| {
            workspace
                .read(cx)
                .chat
                .as_ref()
                .is_none_or(|run| !run.bubble.is_running())
        });
        if done || started.elapsed().as_secs() > 900 {
            break;
        }
    }
    for entry in cx.update(|cx| workspace.read(cx).agent_entries()) {
        println!("{entry:?}");
    }
    let view = map(cx, &workspace).unwrap_or(view);
    let regions = cx.update(|cx| view.read(cx).domain_regions());
    println!(
        "took {:?}, asked {asked} times, proposal: {regions:?}",
        started.elapsed()
    );
    assert_eq!(asked, 0, "the user was asked");
    assert!(cx.update(|cx| view.read(cx).has_proposal()), "a proposal");
}
