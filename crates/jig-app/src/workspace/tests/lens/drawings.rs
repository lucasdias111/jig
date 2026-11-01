//! Drawings in the conversation and on the Map: hiding, touring, pinning, keeping descriptions and proposals.

use super::*;

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
