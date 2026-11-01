//! The conversation's frame: where it opens, moving, resizing and docking it, and ⌘K beside it.

use super::*;

#[gpui_kit::test]
fn the_agent_conversation_moves_by_its_header(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            // Clear of the window's edge, with room to move and grow.
            this.chat.as_mut().unwrap().anchor = gpui_kit::point(px(120.), px(80.));
        })
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let header = vcx
        .debug_bounds("agent-chat-header")
        .expect("the conversation is open");
    let before = vcx.update(|_, cx| workspace.read(cx).chat.as_ref().unwrap().anchor);

    // By its title: the buttons at its end take their own presses.
    let from = gpui_kit::point(header.left() + px(12.), header.center().y);
    let by = gpui_kit::point(px(60.), px(90.));
    let none = gpui_kit::Modifiers::none();
    vcx.simulate_mouse_down(from, gpui_kit::MouseButton::Left, none);
    for i in 1..=5 {
        let at = from + by * (i as f32 / 5.);
        vcx.simulate_mouse_move(at, Some(gpui_kit::MouseButton::Left), none);
    }
    vcx.simulate_mouse_up(from + by, gpui_kit::MouseButton::Left, none);
    vcx.run_until_parked();

    let after = vcx.update(|_, cx| workspace.read(cx).chat.as_ref().unwrap().anchor);
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
            .chat
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
fn the_agent_conversation_opens_right_centre_whatever_the_line(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let short = "fn short() {}\n";
    let long = format!("// {}\n", "x".repeat(110));
    std::fs::write(dir.path().join("a.rs"), format!("{short}{long}")).unwrap();
    let (window, workspace) = open(cx, &dir.path().join("a.rs"));

    let open_on = |cx: &mut TestAppContext, line: std::ops::Range<usize>| {
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            window.render_frame(cx);
            ws.update(cx, |this, cx| {
                this.editor().select(line.clone(), cx);
                this.open_test_conversation(window, cx);
            })
        });
        cx.update(|cx| workspace.read(cx).chat.as_ref().unwrap().anchor)
    };
    let viewport = cx
        .update(|cx| window.update(cx, |_, window, _| window.viewport_size()))
        .unwrap();
    // Centred in the right half, pulled in as far as it must to fit, and
    // top to bottom at the height it grows to, below the title bar.
    let width = px(jig_commands::chat::WIDTH);
    let full = jig_commands::chat::ChatFrame::default().full_height();
    let x = (viewport.width * 0.75 - width / 2.).min(viewport.width - width - px(8.));
    let y = ((viewport.height - full) / 2.).max(px(super::super::super::TITLE_BAR_HEIGHT + 8.));
    for anchor in [
        open_on(cx, 0..short.len()),
        // Code reaching right across: it opens there all the same.
        open_on(cx, short.len()..short.len() + long.len()),
    ] {
        assert!(
            (anchor.x - x).abs() < px(1.) && (anchor.y - y).abs() < px(1.),
            "right-centre: {anchor:?} in {viewport:?}"
        );
    }
}

#[gpui_kit::test]
fn command_k_leaves_the_conversation_open(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.open_test_conversation(window, cx))
    });
    let open = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).chat.is_some());
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-k", cx);
    });
    assert!(cx.update(|cx| workspace.read(cx).palette.is_some()));
    assert!(open(cx), "the palette opens beside it");
    step(cx, window, |window, cx| window.press("escape", cx));
    assert!(open(cx), "and closing it leaves the conversation");
    // A note shows without touching the conversation.
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.show_note("Done.".into(), window, cx))
    });
    assert!(open(cx));
    // Esc dismisses the note first, then closes the conversation.
    step(cx, window, |window, cx| window.press("escape", cx));
    assert!(cx.update(|cx| workspace.read(cx).note.is_none()));
    assert!(open(cx));
    step(cx, window, |window, cx| window.press("escape", cx));
    assert!(!open(cx));
}

#[gpui_kit::test]
fn the_agent_picked_in_command_k_runs_the_next_conversation(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-k", cx);
    });
    let ws = workspace.clone();
    step(cx, window, move |_, cx| {
        ws.update(cx, |this, cx| {
            let palette = this.palette.as_ref().unwrap().view.clone();
            let choice = jig_commands::ModelChoice {
                agent: Some("claude-code".into()),
                group: "Claude Code".into(),
                name: "Opus".into(),
                model: Some("opus".into()),
                label: "Claude Code · Opus".into(),
            };
            this.pick_model(choice, palette, cx);
        })
    });
    assert!(
        cx.update(|cx| workspace.read(cx).palette.is_some()),
        "picking keeps the palette open"
    );
    step(cx, window, |window, cx| window.press("escape", cx));
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.open_test_conversation(window, cx))
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        let agent = this.chat.as_ref().unwrap().agent.as_ref().unwrap();
        assert_eq!(
            agent.pick,
            Some(jig_agent::agent::Pick {
                agent: "claude-code".into(),
                model: Some("opus".into()),
            }),
            "the conversation keeps what was picked"
        );
        assert_eq!(agent.label, "Claude Code");
    });
}

#[gpui_kit::test]
fn the_pin_docks_the_conversation_to_the_right_edge(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.open_test_conversation(window, cx))
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let pin = vcx.debug_bounds("agent-chat-pin").expect("a pin button");
    vcx.simulate_click(pin.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    let header = vcx.debug_bounds("agent-chat-header").unwrap();
    let width = vcx.update(|window, _| window.viewport_size().width);
    assert!(
        width - header.right() < px(40.),
        "against the right edge: {header:?} in {width:?}"
    );
    // Pinned, it stays through the next frame and doesn't float too.
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.update(|_, cx| workspace.read(cx).floating_agent_chat_pinned()));

    // Its left edge resizes it: dragged left, it grows.
    let width = |vcx: &mut gpui_kit::VisualTestContext| {
        vcx.debug_bounds("agent-chat-header").unwrap().size.width
    };
    let before = width(&mut vcx);
    let edge = vcx
        .debug_bounds("agent-dock-resize")
        .expect("a resize edge");
    let none = gpui_kit::Modifiers::none();
    let from = edge.center();
    vcx.simulate_mouse_down(from, gpui_kit::MouseButton::Left, none);
    for i in 1..=4 {
        let at = from - gpui_kit::point(px(25. * i as f32), px(0.));
        vcx.simulate_mouse_move(at, Some(gpui_kit::MouseButton::Left), none);
    }
    vcx.simulate_mouse_up(
        from - gpui_kit::point(px(100.), px(0.)),
        gpui_kit::MouseButton::Left,
        none,
    );
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    let after = width(&mut vcx);
    assert!(
        (after - before - px(100.)).abs() <= px(2.),
        "100 px wider: {before:?} → {after:?}"
    );
    assert!(
        vcx.update(|_, cx| workspace.read(cx).agent.dock.is_some()),
        "the next conversation opens docked, this wide"
    );

    // Docked, ⌘K still opens the input, for another conversation.
    vcx.update(|window, cx| window.press("secondary-k", cx));
    vcx.run_until_parked();
    vcx.update(|_, cx| {
        let this = workspace.read(cx);
        let palette = this.palette.as_ref().expect("the palette opens");
        assert!(this.chat.is_some(), "the docked conversation stays");
        assert!(palette.view.read(cx).query(cx).is_empty());
    });
}

#[gpui_kit::test]
fn the_agent_conversation_resizes_by_its_corner(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            // Clear of the window's edge, with room to move and grow.
            this.chat.as_mut().unwrap().anchor = gpui_kit::point(px(120.), px(80.));
        })
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
