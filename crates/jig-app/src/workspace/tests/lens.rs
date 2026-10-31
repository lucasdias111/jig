//! The agent on the Map: what it draws lands on the boxes holding each
//! place at every level, folded at coarse ones; a drawing's row shows,
//! hides, pins and tours it; ←/→ step through a trace; descriptions are
//! kept with Tab; proposals are dismissed; links fly the Map; ⌘K there
//! tells the agent what's selected and showing.

use jig_ai::agent::AgentEvent;
use jig_commands::DrawingAction;
use serde_json::json;

use super::*;
use jig_map::{Level, MapView};

const STORE: &str = "//! Keeps what's sent.\n\npub fn save(item: u8) -> bool {\n    check(item);\n    write(item)\n}\n\nfn check(item: u8) {}\n\nfn write(item: u8) -> bool {\n    true\n}\n";

/// A small Rust project, `src/store.rs` open with the cursor in `save`,
/// and its Map showing.
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
    (dir, window, workspace, view)
}

/// The agent drew `tool` with `input`, in a conversation on screen.
fn drew(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    tool: &str,
    input: serde_json::Value,
) {
    let ws = workspace.clone();
    let tool = tool.to_string();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            if this.chat.is_none() {
                this.open_test_conversation(window, cx);
            }
            this.test_agent_request(AgentEvent::Drew { tool, input }, window, cx);
            this.focus_code(window, cx);
        })
    });
}

fn trace() -> serde_json::Value {
    json!({"title": "Saving", "steps": [
        {"place": "src/main.rs#main", "label": "Starts"},
        {"place": "src/store.rs#save", "label": "Saves"},
        {"place": "src/store.rs#check", "label": "Checks"},
        {"place": "src/net/client.rs#send", "label": "Sends"},
    ]})
}

fn marks(cx: &mut TestAppContext, view: &Entity<MapView>) -> Vec<(String, Vec<String>)> {
    cx.update(|cx| view.read(cx).lens_marks())
        .into_iter()
        .map(|(key, badges, _)| (key, badges))
        .collect()
}

fn drawings_in_chat(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<ChatEntry> {
    cx.update(|cx| workspace.read(cx).agent_entries())
        .into_iter()
        .filter(|entry| matches!(entry, ChatEntry::Drawing { .. }))
        .collect()
}

#[gpui_kit::test]
fn a_trace_lands_on_the_boxes_holding_it_at_every_level(cx: &mut TestAppContext) {
    let (_dir, window, workspace, view) = on_the_map(cx);
    drew(cx, window, &workspace, "map_trace", trace());
    let rows = drawings_in_chat(cx, &workspace);
    assert!(
        matches!(&rows[..], [ChatEntry::Drawing { summary, shown: true, stepped: true, .. }] if summary == "Trace: Saving · 4 steps"),
        "{rows:?}"
    );
    draw(cx, window);
    // At Plan: each function on it its own stop.
    let at_plan = marks(cx, &view);
    assert!(
        at_plan
            .iter()
            .any(|(key, badges)| key.contains("store.rs#.check") && badges == &["3"]),
        "{at_plan:?}"
    );
    // At Site, open as far as the files: save and check fold onto one.
    press(cx, window, "secondary-up");
    view.update(cx, |view, _| view.end_gesture());
    draw(cx, window);
    let at_site = marks(cx, &view);
    assert!(!at_site.is_empty(), "every step lands somewhere");
    assert!(
        at_site
            .iter()
            .any(|(_, badges)| badges.iter().any(|b| b.contains('–'))),
        "steps in a row on one box fold: {at_site:?}"
    );
    // At Domain, all of it in one region, one stop.
    press(cx, window, "secondary-up");
    view.update(cx, |view, _| view.end_gesture());
    draw(cx, window);
    assert_eq!(read_level(cx, &workspace), Level::Domain);
    let at_domain = marks(cx, &view);
    assert!(
        at_domain.iter().all(|(key, _)| key.starts_with("domain:")),
        "{at_domain:?}"
    );
    // Jig's drafts here: the store and the network apart.
    assert!(
        at_domain
            .iter()
            .any(|(_, badges)| badges.contains(&"1–3".to_string())),
        "{at_domain:?}"
    );
}

fn draw(cx: &mut TestAppContext, window: AnyWindowHandle) {
    step(cx, window, |window, cx| window.render_frame(cx));
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
fn a_name_that_isnt_in_the_project_isnt_drawn(cx: &mut TestAppContext) {
    let (_dir, window, workspace, view) = on_the_map(cx);
    drew(
        cx,
        window,
        &workspace,
        "map_highlight",
        json!({"title": "x", "places": ["src/store.rs#remove"]}),
    );
    assert!(drawings_in_chat(cx, &workspace).is_empty());
    assert_eq!(cx.update(|cx| view.read(cx).drawings_shown()), 0);
    // Svelte files and folders are places too.
    drew(
        cx,
        window,
        &workspace,
        "map_highlight",
        json!({"title": "x", "places": ["web/App.svelte", "src/net/"]}),
    );
    assert_eq!(cx.update(|cx| view.read(cx).drawings_shown()), 1);
}

#[gpui_kit::test]
fn a_drawings_row_hides_it_and_map_clear_clears_it(cx: &mut TestAppContext) {
    let (_dir, window, workspace, view) = on_the_map(cx);
    drew(cx, window, &workspace, "map_trace", trace());
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.act_on_drawing(0, DrawingAction::Toggle, window, cx)
        })
    });
    assert_eq!(cx.update(|cx| view.read(cx).drawings_shown()), 0);
    assert!(matches!(
        &drawings_in_chat(cx, &workspace)[..],
        [ChatEntry::Drawing { shown: false, .. }]
    ));
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.act_on_drawing(0, DrawingAction::Toggle, window, cx)
        })
    });
    assert_eq!(cx.update(|cx| view.read(cx).drawings_shown()), 1);
    drew(cx, window, &workspace, "map_clear", json!({}));
    assert_eq!(cx.update(|cx| view.read(cx).drawings_shown()), 0);
    // Closing the conversation takes its drawings with it.
    drew(cx, window, &workspace, "map_trace", trace());
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.close_chat(window, cx))
    });
    assert_eq!(cx.update(|cx| view.read(cx).drawings_shown()), 0);
}

#[gpui_kit::test]
fn arrows_tour_a_trace_and_the_camera_follows(cx: &mut TestAppContext) {
    let (_dir, window, workspace, view) = on_the_map(cx);
    drew(cx, window, &workspace, "map_trace", trace());
    press(cx, window, "right");
    assert_eq!(cx.update(|cx| view.read(cx).tour_step()), Some(0));
    // Step 1 is main, on save's Plan as its caller: the camera goes to
    // its box there.
    let selected = cx.update(|cx| view.read(cx).selected_keys().to_vec());
    assert!(
        selected.iter().any(|key| key.contains("main.rs#.main")),
        "{selected:?}"
    );
    press(cx, window, "right");
    assert_eq!(cx.update(|cx| view.read(cx).tour_step()), Some(1));
    press(cx, window, "right");
    press(cx, window, "right");
    // Step 4 is send, not on save's Plan: the tour goes to its Plan.
    let send = |level: &Level| matches!(level, Level::Plan(jig_map::Focus::Function(s)) if s.name == "send");
    assert!(
        send(&read_level(cx, &workspace)),
        "{:?}",
        read_level(cx, &workspace)
    );
    press(cx, window, "left");
    assert_eq!(cx.update(|cx| view.read(cx).tour_step()), Some(2));
    press(cx, window, "escape");
    assert_eq!(cx.update(|cx| view.read(cx).tour_step()), None);
    assert!(
        map_is_showing(cx, &workspace),
        "Esc ended the tour, not the Map"
    );
}

fn map_is_showing(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> bool {
    cx.update(|cx| workspace.read(cx).map_showing().is_some())
}

#[gpui_kit::test]
fn a_pinned_drawing_is_saved_and_listed_when_orphaned(cx: &mut TestAppContext) {
    let (dir, window, workspace, view) = on_the_map(cx);
    drew(cx, window, &workspace, "map_trace", trace());
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.act_on_drawing(0, DrawingAction::Pin, window, cx)
        })
    });
    let names = cx.update(|cx| view.read(cx).pin_names());
    assert_eq!(names, ["trace-saving-4-steps"]);
    let pin = dir.path().join(".jig/map/pins/trace-saving-4-steps.toml");
    let text = std::fs::read_to_string(&pin).unwrap();
    assert!(text.contains("place = \"src/store.rs#save\""), "{text}");
    assert!(matches!(
        &drawings_in_chat(cx, &workspace)[..],
        [ChatEntry::Drawing { pinned: true, .. }]
    ));
    // Its conversation gone, the pin stays on its layer.
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.close_chat(window, cx))
    });
    assert_eq!(cx.update(|cx| view.read(cx).drawings_shown()), 1);
    view.update(cx, |view, cx| view.set_pins_shown(false, cx));
    assert_eq!(cx.update(|cx| view.read(cx).drawings_shown()), 0);
    // A place gone: the pin is orphaned.
    std::fs::write(
        &pin,
        text.replace("src/net/client.rs#send", "src/net/client.rs#gone"),
    )
    .unwrap();
    let path = dir.path().join("src/store.rs");
    view.update(cx, |view, cx| {
        let project = jig_map::Project::read_again(path.parent().unwrap().parent().unwrap(), None);
        view.set_project(std::sync::Arc::new(project), cx);
    });
    let orphans = cx.update(|cx| view.read(cx).orphaned_pins());
    assert_eq!(orphans, [("trace-saving-4-steps".to_string(), 1)]);
}

#[gpui_kit::test]
fn tab_keeps_the_agents_descriptions(cx: &mut TestAppContext) {
    let (dir, window, workspace, view) = on_the_map(cx);
    press(cx, window, "secondary-up");
    view.update(cx, |view, _| view.end_gesture());
    drew(
        cx,
        window,
        &workspace,
        "map_describe",
        json!({"purposes": [{"place": "src/net/", "purpose": "Talks to the server"}]}),
    );
    let words = cx.update(|cx| view.read(cx).lens_marks());
    assert!(
        words
            .iter()
            .any(|(_, _, words)| words.contains(&"Purpose: Talks to the server".to_string())),
        "{words:?}"
    );
    press(cx, window, "tab");
    let kept = std::fs::read_to_string(dir.path().join(".jig/map/purposes.toml")).unwrap();
    assert!(
        kept.contains("\"src/net/\" = \"Talks to the server\""),
        "{kept}"
    );
    assert!(matches!(
        &drawings_in_chat(cx, &workspace)[..],
        [ChatEntry::Drawing { shown: false, settled: Some(word), .. }] if word == "Kept"
    ));
    let site = cx.update(|cx| {
        let view = view.read(cx);
        let site = view.site().unwrap();
        site.nodes
            .iter()
            .find(|node| node.place == "src/net/")
            .and_then(|node| node.purpose.clone())
    });
    assert_eq!(site.as_deref(), Some("Talks to the server"));
}

#[gpui_kit::test]
fn a_proposal_is_dismissed(cx: &mut TestAppContext) {
    let (_dir, window, workspace, view) = on_the_map(cx);
    drew(
        cx,
        window,
        &workspace,
        "map_propose",
        json!({"title": "A cache", "boxes": [{"name": "Cache", "near": "src/store.rs#save", "purpose": "Keeps the last item"}],
               "arrows": [{"from": "src/store.rs#save", "to": "Cache"}]}),
    );
    draw(cx, window);
    assert_eq!(cx.update(|cx| view.read(cx).drawings_shown()), 1);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.proposal_chosen(0, jig_map::ProposalChoice::Dismiss, window, cx)
        })
    });
    assert_eq!(cx.update(|cx| view.read(cx).drawings_shown()), 0);
    assert!(matches!(
        &drawings_in_chat(cx, &workspace)[..],
        [ChatEntry::Drawing { settled: Some(word), .. }] if word == "Dismissed"
    ));
}

#[gpui_kit::test]
fn a_link_in_the_answer_flies_the_map_there(cx: &mut TestAppContext) {
    let (_dir, window, workspace, view) = on_the_map(cx);
    press(cx, window, "secondary-up");
    view.update(cx, |view, _| view.end_gesture());
    drew(cx, window, &workspace, "map_trace", trace());
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_agent_link("src/net/client.rs#send", window, cx)
        })
    });
    let selected = cx.update(|cx| view.read(cx).selected_keys().to_vec());
    assert!(
        selected.iter().any(|key| key.contains("client.rs")),
        "{selected:?}"
    );
    assert!(map_is_showing(cx, &workspace));
}

#[gpui_kit::test]
fn command_k_on_the_map_tells_the_agent_what_is_selected(cx: &mut TestAppContext) {
    let (_dir, window, workspace, _view) = on_the_map(cx);
    press(cx, window, "secondary-k");
    assert!(cx.update(|cx| workspace.read(cx).palette.is_some()));
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("How does saving work?", cx);
    });
    press(cx, window, "enter");
    let prompt = cx.update(|cx| workspace.read(cx).agent_prompt()).unwrap();
    assert!(prompt.starts_with("How does saving work?"), "{prompt}");
    assert!(
        prompt.contains("I'm in Jig's Map of this project, at its Plan level"),
        "{prompt}"
    );
    assert!(
        prompt.contains("Selected:\n- src/store.rs#save"),
        "{prompt}"
    );
    assert!(prompt.contains("src/store.rs:3"), "where it is: {prompt}");
    assert!(
        prompt.contains("save calls") || prompt.contains("calls src/store.rs#check"),
        "{prompt}"
    );
    assert!(!prompt.contains("check(item);"), "no code is sent");
    assert_eq!(
        jig_ai::agent::asked(&prompt),
        "How does saving work?",
        "the transcript shows only the question"
    );
}

/// Asks a real agent on the Map of a real project and saves pictures of
/// its answer: `JIG_MAP_ROOT=<project> JIG_MAP_FILE=<file in it>
/// JIG_MAP_ASK=<question> JIG_PICTURES=<folder> cargo test -p jig-app
/// map_agent_live -- --ignored --nocapture`, after `cargo build -p
/// jig-app` (the agent starts that build's `--map-mcp`).
#[gpui_kit::test]
#[ignore = "needs OpenCode, a provider and network access"]
fn map_agent_live(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let root = std::path::PathBuf::from(std::env::var("JIG_MAP_ROOT").unwrap());
    let file = std::path::PathBuf::from(std::env::var("JIG_MAP_FILE").unwrap());
    let ask = std::env::var("JIG_MAP_ASK").unwrap();
    let out = std::path::PathBuf::from(std::env::var("JIG_PICTURES").unwrap());
    std::fs::create_dir_all(&out).unwrap();
    let jig = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/Jig");
    jig_ai::tasks::mcp::set_command(jig.canonicalize().unwrap());
    let (window, workspace) = open(cx, &root);
    // The user's own agent and providers.
    cx.update(|cx| {
        jig_settings::init(cx);
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
    // Out to Domain, where the whole answer shows.
    for _ in 0..2 {
        press(cx, window, "secondary-up");
        view.update(cx, |view, _| view.end_gesture());
    }
    press(cx, window, "secondary-k");
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        window.input(&ask, cx);
    });
    press(cx, window, "enter");
    let started = std::time::Instant::now();
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        let done = cx.update(|cx| {
            workspace
                .read(cx)
                .chat
                .as_ref()
                .is_none_or(|run| !run.bubble.is_running())
        });
        if done || started.elapsed().as_secs() > 300 {
            break;
        }
    }
    let entries = cx.update(|cx| workspace.read(cx).agent_entries());
    for entry in &entries {
        println!("{entry:?}");
    }
    println!("took {:?}", started.elapsed());
    draw(cx, window);
    for (name, bytes) in view.update(cx, |view, cx| view.pictures(cx)) {
        let path = out.join(format!("domain-{name}.png"));
        std::fs::write(&path, bytes).unwrap();
        println!("wrote {}", path.display());
    }
    press(cx, window, "secondary-down");
    view.update(cx, |view, _| view.end_gesture());
    draw(cx, window);
    if let Some(bytes) = view.update(cx, |view, cx| view.picture_now(cx)) {
        std::fs::write(out.join("site.png"), bytes).unwrap();
    }
    assert!(
        entries
            .iter()
            .any(|entry| matches!(entry, ChatEntry::Drawing { .. })),
        "the agent drew on the Map"
    );
}

#[gpui_kit::test]
fn the_list_keeps_clear_of_the_conversation_and_the_guide_steps(cx: &mut TestAppContext) {
    let (_dir, window, workspace, view) = on_the_map(cx);
    drew(cx, window, &workspace, "map_trace", trace());
    draw(cx, window);
    // The conversation drawn over the canvas's top left: the list goes
    // top right.
    let canvas = cx.update(|cx| view.read(cx).canvas_bounds());
    let over = std::rc::Rc::new(std::cell::Cell::new(gpui_kit::Bounds {
        origin: canvas.origin,
        size: gpui_kit::size(px(200.), px(200.)),
    }));
    view.update(cx, |view, _| {
        view.set_conversation_bounds(Some(over.clone()))
    });
    assert!(cx.update(|cx| view.read(cx).list_goes_right()));
    over.set(gpui_kit::Bounds {
        origin: canvas.origin + gpui_kit::point(canvas.size.width - px(100.), px(0.)),
        size: gpui_kit::size(px(100.), px(100.)),
    });
    assert!(!cx.update(|cx| view.read(cx).list_goes_right()));
    // The guide's Next and Back are the arrow keys' steps.
    press(cx, window, "right");
    view.update(cx, |view, cx| {
        view.tour_by(1, cx);
        view.tour_by(1, cx);
        view.tour_by(-1, cx);
    });
    assert_eq!(cx.update(|cx| view.read(cx).tour_step()), Some(1));
}
