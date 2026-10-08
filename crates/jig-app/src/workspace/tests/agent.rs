//! The agent conversation and a live agent run.

use super::*;

#[gpui_kit::test]
fn the_agent_conversation_moves_by_its_header(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.open_test_conversation(window, cx))
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let header = vcx
        .debug_bounds("agent-chat-header")
        .expect("the conversation is open");
    let before = vcx.update(|_, cx| workspace.read(cx).run.as_ref().unwrap().anchor);

    let (from, by) = (header.center(), gpui_kit::point(px(60.), px(90.)));
    let none = gpui_kit::Modifiers::none();
    vcx.simulate_mouse_down(from, gpui_kit::MouseButton::Left, none);
    for i in 1..=5 {
        let at = from + by * (i as f32 / 5.);
        vcx.simulate_mouse_move(at, Some(gpui_kit::MouseButton::Left), none);
    }
    vcx.simulate_mouse_up(from + by, gpui_kit::MouseButton::Left, none);
    vcx.run_until_parked();

    let after = vcx.update(|_, cx| workspace.read(cx).run.as_ref().unwrap().anchor);
    // Within a pixel: positions are snapped to whole pixels when drawn.
    let near = |a: gpui_kit::Point<gpui_kit::Pixels>, b: gpui_kit::Point<gpui_kit::Pixels>| {
        (a.x - b.x).abs() <= px(1.) && (a.y - b.y).abs() <= px(1.)
    };
    assert!(
        near(after, before + by),
        "it moved with the mouse: {after:?}"
    );
    vcx.update(|window, cx| window.render_frame(cx));
    let moved = vcx.debug_bounds("agent-chat-header").unwrap();
    // The panel may still be settling into place from its slide-in.
    let offset = moved.origin - header.origin;
    assert!(
        (offset.x - by.x).abs() <= px(1.) && (offset.y - by.y).abs() <= px(7.),
        "drawn where it was dropped: {offset:?}"
    );
}

#[gpui_kit::test]
fn clicking_anywhere_in_the_reply_box_focuses_it(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.open_test_conversation(window, cx))
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let reply = vcx
        .debug_bounds("agent-reply-box")
        .expect("the reply box is shown between turns");
    // In the box's padding, well clear of the text.
    let edge = gpui_kit::point(reply.right() - px(3.), reply.center().y);
    vcx.simulate_click(edge, gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|window, cx| {
        let this = workspace.read(cx);
        let input = this
            .run
            .as_ref()
            .unwrap()
            .agent
            .as_ref()
            .unwrap()
            .input
            .clone();
        assert!(input.focus_handle(cx).is_focused(window));
    });
}

#[gpui_kit::test]
fn the_agent_conversation_opens_beside_the_code_when_it_fits(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let short = "fn short() {}\n";
    let long = format!("// {}\n", "x".repeat(110));
    std::fs::write(dir.path().join("a.rs"), format!("{short}{long}")).unwrap();
    let (window, workspace) = open(cx, &dir.path().join("a.rs"));

    // Open on `line`, and say where it ended up against that line's end.
    let open_on = |cx: &mut TestAppContext, line: std::ops::Range<usize>| {
        let (ws, selected) = (workspace.clone(), line.clone());
        step(cx, window, move |window, cx| {
            window.render_frame(cx);
            ws.update(cx, |this, cx| {
                this.editor().select(selected.clone(), cx);
                this.open_test_conversation(window, cx);
            })
        });
        cx.update(|cx| {
            let this = workspace.read(cx);
            let end = this
                .editor()
                .state()
                .read(cx)
                .range_to_bounds(&(line.end - 1..line.end - 1))
                .unwrap();
            (this.run.as_ref().unwrap().anchor, end)
        })
    };

    let (anchor, end) = open_on(cx, 0..short.len());
    assert!(
        anchor.x > end.left(),
        "right of the code: {anchor:?} {end:?}"
    );
    assert!((anchor.y - end.top()).abs() < px(8.), "level with it");

    let (anchor, end) = open_on(cx, short.len()..short.len() + long.len());
    assert!(
        anchor.y >= end.bottom(),
        "no room on the right, so below it: {anchor:?} {end:?}"
    );
}

#[gpui_kit::test]
fn the_agent_conversation_resizes_by_its_corner(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.open_test_conversation(window, cx))
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let grip = vcx
        .debug_bounds("agent-chat-resize")
        .expect("a corner grip");
    let header = vcx.debug_bounds("agent-chat-header").unwrap();

    let (from, by) = (grip.center(), gpui_kit::point(px(80.), px(60.)));
    let none = gpui_kit::Modifiers::none();
    vcx.simulate_mouse_down(from, gpui_kit::MouseButton::Left, none);
    for i in 1..=5 {
        let at = from + by * (i as f32 / 5.);
        vcx.simulate_mouse_move(at, Some(gpui_kit::MouseButton::Left), none);
    }
    vcx.simulate_mouse_up(from + by, gpui_kit::MouseButton::Left, none);
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));

    let wider = vcx.debug_bounds("agent-chat-header").unwrap();
    assert_eq!(
        wider.size.width,
        header.size.width + by.x,
        "wider by the drag"
    );
    assert_eq!(
        wider.origin.x, header.origin.x,
        "growing from its left edge"
    );
    let moved = vcx.debug_bounds("agent-chat-resize").unwrap().origin - grip.origin;
    assert!(
        (moved.x - by.x).abs() <= px(1.) && (moved.y - by.y).abs() <= px(7.),
        "the corner followed the mouse: {moved:?}"
    );
}

/// Wait in real time for the agent, letting the window handle what it
/// sends, until `done` holds.
fn wait_for_agent(
    cx: &mut TestAppContext,
    workspace: &Entity<Workspace>,
    what: &str,
    done: impl Fn(&Option<Bubble>) -> bool,
) -> Option<Bubble> {
    for _ in 0..240 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(500));
        cx.run_until_parked();
        let bubble = bubble(cx, workspace);
        if done(&bubble) {
            return bubble;
        }
    }
    panic!("timed out waiting for {what}: {:?}", bubble(cx, workspace));
}

/// Talks to a real OpenCode: `cargo test -p jig-app agent_live -- --ignored`.
#[gpui_kit::test]
#[ignore = "needs OpenCode and network access"]
fn agent_live_edit_is_reviewed_then_written(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n").unwrap();
    let (window, workspace) = open(cx, &path);

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-k", cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("tab", cx);
        window.input("Add a one-line doc comment to add. Nothing else.", cx);
        window.press("enter", cx);
    });
    let started = bubble(cx, &workspace);
    assert!(
        matches!(started, Some(Bubble::Running { .. })),
        "{started:?}"
    );

    let preview = wait_for_agent(cx, &workspace, "an edit", |b| {
        matches!(b, Some(Bubble::Preview { .. } | Bubble::Error(_)))
    });
    assert!(
        matches!(preview, Some(Bubble::Preview { .. })),
        "{preview:?}"
    );
    assert!(
        text(cx, &workspace).contains("///"),
        "the edit is shown in the buffer"
    );
    assert!(
        !std::fs::read_to_string(&path).unwrap().contains("///"),
        "nothing is written before the user accepts"
    );

    step(cx, window, |window, cx| window.press("enter", cx));
    let done = wait_for_agent(cx, &workspace, "the agent to finish", |b| {
        matches!(
            b,
            Some(Bubble::AgentDone(_) | Bubble::Error(_) | Bubble::Preview { .. })
        )
    });
    assert!(matches!(done, Some(Bubble::AgentDone(_))), "{done:?}");
    let disk = std::fs::read_to_string(&path).unwrap();
    assert!(disk.contains("///"), "accepted edit is on disk: {disk}");
    assert_eq!(text(cx, &workspace), disk);
    assert!(!cx.update(|cx| workspace.read(cx).tab().dirty));
    eprintln!("agent said: {done:?}");

    // The reply box has the keyboard; a reply continues the session.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("Which function did you document? Answer, don't edit.", cx);
        window.press("enter", cx);
    });
    assert!(matches!(
        bubble(cx, &workspace),
        Some(Bubble::Running { .. })
    ));
    let answer = wait_for_agent(cx, &workspace, "the reply", |b| {
        matches!(b, Some(Bubble::AgentDone(_) | Bubble::Error(_)))
    });
    assert!(matches!(answer, Some(Bubble::AgentDone(_))), "{answer:?}");
    let entries = cx.update(|cx| workspace.read(cx).agent_entries());
    assert!(
        matches!(
            entries.as_slice(),
            [
                ChatEntry::User(_),
                ChatEntry::Edit { accepted: true, .. },
                ChatEntry::Agent(_),
                ChatEntry::User(_),
                ChatEntry::Agent(_),
            ]
        ),
        "{entries:?}"
    );
    eprintln!("agent answered: {answer:?}");
    crate::agent::stop();
}
