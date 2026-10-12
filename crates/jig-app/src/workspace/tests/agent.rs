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
            (this.chat.as_ref().unwrap().anchor, end)
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

fn permission(detail: &str) -> jig_ai::agent::AgentEvent {
    jig_ai::agent::AgentEvent::Permission(jig_ai::agent::PermissionRequest {
        id: "per_1".into(),
        title: "Run a command".into(),
        detail: detail.into(),
        always: None,
    })
}

#[gpui_kit::test]
fn a_command_runs_only_once_allowed(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            this.test_agent_request(permission("cargo test"), window, cx);
        })
    });
    let waiting = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).agent_waiting_on());
    assert_eq!(waiting(cx), Some("permission"));
    // Enter, back in the code, allows it.
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(waiting(cx), None);
    assert!(
        cx.update(|cx| workspace.read(cx).editor().text(cx))
            .starts_with("// a"),
        "Enter went to the request, not into the file"
    );

    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.test_agent_request(permission("rm -rf target"), window, cx)
        })
    });
    // Esc turns it down and keeps the conversation.
    step(cx, window, |window, cx| window.press("escape", cx));
    assert_eq!(waiting(cx), None);
    let entries = cx.update(|cx| workspace.read(cx).agent_entries());
    assert_eq!(
        entries.last(),
        Some(&ChatEntry::Note("Not allowed: rm -rf target".into()))
    );
}

#[gpui_kit::test]
fn always_allowing_stops_the_asking_for_the_conversation(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            this.test_agent_request(permission("cargo test"), window, cx);
        })
    });
    let waiting = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).agent_waiting_on());
    // The buttons are drawn, and the middle one allows it always.
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let always = vcx
        .debug_bounds("agent-decide-1")
        .expect("an Always allow button");
    assert!(
        vcx.debug_bounds("agent-decide-0").is_some()
            && vcx.debug_bounds("agent-decide-2").is_some()
    );
    vcx.simulate_click(always.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(waiting(cx), None);
    let entries = cx.update(|cx| workspace.read(cx).agent_entries());
    assert_eq!(
        entries.last(),
        Some(&ChatEntry::Note(
            "Run a command: allowed for this conversation.".into()
        ))
    );

    // The next command goes through without asking; the agent can't
    // remember, so Jig does.
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.test_agent_request(permission("cargo build"), window, cx)
        })
    });
    assert_eq!(waiting(cx), None, "not asked again");

    // ⌘↩ in the code allows always too, rather than opening a line.
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            let mut asking = permission("ls");
            if let jig_ai::agent::AgentEvent::Permission(request) = &mut asking {
                request.title = "Use the web".into();
                request.always = Some("Always allow `ls`".into());
            }
            this.test_agent_request(asking, window, cx)
        })
    });
    assert_eq!(waiting(cx), Some("permission"));
    let text = cx.update(|cx| workspace.read(cx).editor().text(cx));
    step(cx, window, |window, cx| window.press("secondary-enter", cx));
    assert_eq!(waiting(cx), None);
    assert_eq!(cx.update(|cx| workspace.read(cx).editor().text(cx)), text);
    let entries = cx.update(|cx| workspace.read(cx).agent_entries());
    assert_eq!(
        entries.last(),
        Some(&ChatEntry::Note("Always allow `ls`.".into()))
    );
}

#[gpui_kit::test]
fn the_agent_s_question_is_answered_by_number_or_in_words(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let question = |text: &str| jig_ai::agent::Question {
        question: text.into(),
        options: vec!["Yes".into(), "No".into()],
    };
    let ask = jig_ai::agent::AgentEvent::Question(jig_ai::agent::QuestionRequest {
        id: "que_1".into(),
        questions: vec![question("Keep the old name?"), question("Add a test?")],
    });
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            this.test_agent_request(ask, window, cx);
        })
    });
    // The reply box has the keyboard.
    step(cx, window, |window, cx| window.input("2", cx));
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(
        cx.update(|cx| workspace.read(cx).agent_waiting_on()),
        Some("question"),
        "one more to go"
    );
    step(cx, window, |window, cx| {
        window.input("only if it's quick", cx)
    });
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(cx.update(|cx| workspace.read(cx).agent_waiting_on()), None);
    let entries = cx.update(|cx| workspace.read(cx).agent_entries());
    assert_eq!(
        entries.last(),
        Some(&ChatEntry::User("No · only if it's quick".into()))
    );
}

#[gpui_kit::test]
fn an_agent_that_wants_a_sign_in_offers_its_ways(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let needed = jig_ai::acp::SignInNeeded {
        agent: "Claude Code".into(),
        methods: vec![jig_ai::acp::SignIn {
            id: "claude-ai-login".into(),
            name: "Claude Subscription".into(),
            description: String::new(),
            terminal: None,
        }],
    };
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            this.test_sign_in(needed, window, cx);
        })
    });
    assert_eq!(
        cx.update(|cx| workspace.read(cx).agent_waiting_on()),
        Some("sign-in")
    );
    let entries = cx.update(|cx| workspace.read(cx).agent_entries());
    assert_eq!(
        entries.last(),
        Some(&ChatEntry::Note("Claude Code needs you to sign in.".into()))
    );
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
}

#[gpui_kit::test]
fn the_conversation_belongs_to_the_project_not_the_tab(cx: &mut TestAppContext) {
    let (dir, window, workspace) = three_files(cx);
    let b = dir.path().join("b.rs");
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            // The agent proposes an edit to b.rs, which opens there.
            let edit = jig_ai::agent::EditRequest {
                id: "per_edit".into(),
                path: b.clone(),
                change: jig_ai::agent::EditChange::Write("// b.rs, edited\n".into()),
            };
            this.test_agent_request(jig_ai::agent::AgentEvent::Edit(edit), window, cx);
        })
    });
    let file = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let this = workspace.read(cx);
            this.document()
                .path
                .as_ref()
                .and_then(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
        })
    };
    assert_eq!(file(cx).as_deref(), Some("b.rs"));
    assert_eq!(
        cx.update(|cx| workspace.read(cx).agent_waiting_on()),
        Some("edit")
    );

    // To another file: the conversation stays, the edit waits in b.rs, and
    // this file isn't locked by it.
    let a = dir.path().join("a.rs");
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.open_file(&a, window, cx))
    });
    assert_eq!(file(cx).as_deref(), Some("a.rs"));
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(
            this.chat.as_ref().is_some_and(|run| run.agent.is_some()),
            "still open"
        );
        assert!(!this.previewing(), "a.rs isn't under review");
    });
    assert_eq!(
        cx.update(|cx| workspace.read(cx).agent_waiting_on()),
        Some("edit")
    );

    // Esc goes back to the edit rather than stopping the agent; then Enter
    // takes it.
    step(cx, window, |window, cx| window.press("escape", cx));
    assert_eq!(file(cx).as_deref(), Some("b.rs"));
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(cx.update(|cx| workspace.read(cx).agent_waiting_on()), None);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.rs")).unwrap(),
        "// b.rs, edited\n",
        "Jig wrote it, as the agent asked"
    );
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
    // A note from ⌘K's side shows without touching the conversation.
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.show_note("Done.".into(), window, cx))
    });
    assert!(open(cx));
    // Esc dismisses the note first, then closes the conversation.
    step(cx, window, |window, cx| window.press("escape", cx));
    assert!(cx.update(|cx| workspace.read(cx).run.is_none()));
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
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            let palette = this.palette.as_ref().unwrap().view.clone();
            let choice = jig_commands::ModelChoice {
                agent: Some("claude-code".into()),
                group: "Claude Code".into(),
                name: "Opus".into(),
                model: Some("opus".into()),
                label: "Claude Code · Opus".into(),
            };
            this.pick_model(choice, palette, window, cx);
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
            Some(crate::agent::Pick {
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
        vcx.update(|_, cx| workspace.read(cx).agent_dock.is_some()),
        "the next conversation opens docked, this wide"
    );

    // Docked, ⌘K opens the palette on Jig, with Agent mode in the panel.
    vcx.update(|window, cx| window.press("secondary-k", cx));
    vcx.run_until_parked();
    vcx.update(|_, cx| {
        let this = workspace.read(cx);
        let palette = this.palette.as_ref().expect("the palette opens");
        assert!(this.chat.is_some(), "the docked conversation stays");
        assert!(palette.view.read(cx).is_jig_only());
    });
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
        let bubble = chat_bubble(cx, workspace);
        if done(&bubble) {
            return bubble;
        }
    }
    panic!(
        "timed out waiting for {what}: {:?}",
        chat_bubble(cx, workspace)
    );
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
    let started = chat_bubble(cx, &workspace);
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
        chat_bubble(cx, &workspace),
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

/// The agent conversation's status, which it keeps apart from ⌘K's bubble.
fn chat_bubble(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Option<Bubble> {
    cx.update(|cx| {
        workspace
            .read(cx)
            .chat
            .as_ref()
            .map(|chat| chat.bubble.clone())
    })
}

/// Claude Code chosen in Settings, with the models it said it offers.
fn with_claude_code(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::settings::update(cx, |s| s.agent.using = "claude-code".into());
        crate::agents::remember_models(
            "claude-code",
            jig_ai::acp::AgentModels {
                config_id: "model".into(),
                current: "default".into(),
                options: vec![
                    ("default".into(), "Default (recommended)".into()),
                    ("opus".into(), "Opus 5.5".into()),
                ],
            },
        );
    });
}

#[gpui_kit::test]
fn command_k_offers_claude_codes_models_in_agent_mode(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    with_claude_code(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-k", cx);
    });
    step(cx, window, |window, cx| window.press("tab", cx));
    let labels = cx.update(|cx| {
        let (_, agent) = workspace.read(cx).model_menus(cx);
        agent
            .choices
            .iter()
            .map(|choice| choice.label.clone())
            .collect::<Vec<_>>()
    });
    assert!(
        labels.iter().any(|label| label == "Claude Code · Opus 5.5"),
        "{labels:?}"
    );
}

#[gpui_kit::test]
fn the_conversation_s_header_switches_its_model(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    with_claude_code(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.open_test_conversation(window, cx))
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let chip = vcx
        .debug_bounds("agent-chat-model")
        .expect("the model shows in the header");
    vcx.simulate_click(chip.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    let opus = vcx
        .debug_bounds("agent-model-1")
        .expect("its models are listed");
    vcx.simulate_click(opus.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|_, cx| {
        let agent = workspace
            .read(cx)
            .chat
            .as_ref()
            .unwrap()
            .agent
            .as_ref()
            .unwrap();
        assert_eq!(agent.pick.as_ref().unwrap().model.as_deref(), Some("opus"));
        assert_eq!(
            agent.entries.last(),
            Some(&ChatEntry::Note("Now on Opus 5.5.".into()))
        );
    });
}

#[gpui_kit::test]
fn a_long_reply_shows_its_end_and_says_when_there_is_more_below(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    // Over 4 KB, which the Markdown view parses in the background.
    let long = (1..=150)
        .map(|n| format!("Paragraph {n} of a long answer."))
        .collect::<Vec<_>>()
        .join("\n\n");
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.open_test_conversation(window, cx))
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    // Drawn before the Markdown is parsed, as on screen: the reply is laid
    // out short at first.
    let ws = workspace.clone();
    vcx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.test_agent_request(jig_ai::agent::AgentEvent::Done(long), window, cx)
        });
        window.render_frame(cx);
    });
    for _ in 0..3 {
        vcx.update(|window, cx| window.render_frame(cx));
        vcx.run_until_parked();
    }
    let scroll = |vcx: &mut gpui_kit::VisualTestContext| {
        vcx.update(|_, cx| workspace.read(cx).agent_scroll())
    };
    let (down, max) = scroll(&mut vcx);
    assert!(max > px(0.), "the reply overflows the transcript");
    assert_eq!(down, max, "it ends scrolled to the reply's last words");
    assert!(vcx.debug_bounds("agent-more-below").is_none());

    // Scrolled up, it stays put and offers the way back down.
    let transcript = vcx.debug_bounds("agent-chat-header").unwrap();
    let at = gpui_kit::point(transcript.center().x, transcript.bottom() + px(80.));
    vcx.simulate_event(gpui_kit::ScrollWheelEvent {
        position: at,
        delta: gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(120.))),
        ..Default::default()
    });
    for _ in 0..2 {
        vcx.update(|window, cx| window.render_frame(cx));
        vcx.run_until_parked();
    }
    let (down, max) = scroll(&mut vcx);
    assert!(down < max, "scrolled up: {down:?} of {max:?}");
    let more = vcx
        .debug_bounds("agent-more-below")
        .expect("it says there's more below");
    vcx.simulate_click(more.center(), gpui_kit::Modifiers::none());
    for _ in 0..2 {
        vcx.update(|window, cx| window.render_frame(cx));
        vcx.run_until_parked();
    }
    let (down, max) = scroll(&mut vcx);
    assert_eq!(down, max, "back at the end");
}

#[gpui_kit::test]
fn tool_calls_in_a_row_fold_into_their_count(cx: &mut TestAppContext) {
    use jig_ai::agent::AgentEvent;
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            for call in ["Ran `grep -n foo`", "Ran `sed -n 1,9p a.rs`", "Read a.rs"] {
                this.test_agent_request(AgentEvent::Did(call.into()), window, cx);
            }
            this.test_agent_request(
                AgentEvent::Did("Edited b.rs without asking".into()),
                window,
                cx,
            );
            this.test_agent_request(AgentEvent::Did("Ran `cargo test`".into()), window, cx);
        })
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    // The three calls after "Hello" fold into one row; the edit made
    // without asking stays in sight, and the lone call after it shows as is.
    let folded = vcx
        .debug_bounds("agent-tools-1")
        .expect("the calls fold into their count");
    assert!(vcx.debug_bounds("agent-tools-5").is_none());
    assert_eq!(
        vcx.update(|_, cx| workspace.read(cx).agent_entries())[4],
        ChatEntry::Note("Edited b.rs without asking".into())
    );

    assert!(vcx.debug_bounds("agent-tools-1-list").is_none());
    // A click lists them, and another folds them again.
    vcx.simulate_click(folded.center(), gpui_kit::Modifiers::none());
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("agent-tools-1-list").is_some());
    let folded = vcx.debug_bounds("agent-tools-1").unwrap();
    vcx.simulate_click(folded.center(), gpui_kit::Modifiers::none());
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("agent-tools-1-list").is_none());
}

#[gpui_kit::test]
fn escape_closes_the_docked_conversation(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            this.test_agent_request(
                jig_ai::agent::AgentEvent::Done("All done, see below.".into()),
                window,
                cx,
            );
        })
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let pin = vcx.debug_bounds("agent-chat-pin").expect("a pin button");
    vcx.simulate_click(pin.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.update(|_, cx| workspace.read(cx).chat.is_some()));
    // Working in the sidebar beside it: the Tasks view has the keyboard.
    vcx.update(|window, cx| window.press("ctrl-shift-t", cx));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.update(|window, cx| workspace.read(cx).planner_focused(window, cx)));
    vcx.update(|window, cx| window.press("escape", cx));
    vcx.run_until_parked();
    assert!(
        vcx.update(|_, cx| workspace.read(cx).chat.is_none()),
        "Esc closes it, from the sidebar too"
    );
    assert!(
        vcx.update(|window, cx| workspace
            .read(cx)
            .editor()
            .state()
            .focus_handle(cx)
            .is_focused(window)),
        "and the code has the keyboard"
    );
}
