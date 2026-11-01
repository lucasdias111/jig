//! Drawing sketches with the tools, and asking the agent about them.

use super::*;

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
