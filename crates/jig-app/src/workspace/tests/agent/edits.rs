//! The agent's edits: reviewed one at a time, accepted together, settled with buttons, opened to their diff.

use super::*;

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
    jig_agent::agent::stop();
}

#[gpui_kit::test]
fn command_enter_accepts_the_agent_s_edits_for_the_conversation(cx: &mut TestAppContext) {
    use jig_ai::agent::{AgentEvent, EditChange, EditRequest};
    let (dir, window, workspace) = three_files(cx);
    let write = |id: &str, name: &str, text: &str| {
        AgentEvent::Edit(EditRequest {
            id: id.into(),
            path: dir.path().join(name),
            change: EditChange::Write(text.into()),
        })
    };
    let first = write("e1", "a.rs", "// a, edited\n");
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            this.test_agent_request(first, window, cx);
        })
    });
    let waiting = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).agent_waiting_on());
    assert_eq!(waiting(cx), Some("edit"), "the first one is reviewed");

    step(cx, window, |window, cx| window.press("secondary-enter", cx));
    assert_eq!(waiting(cx), None);
    let read = |name: &str| std::fs::read_to_string(dir.path().join(name)).unwrap();
    assert_eq!(read("a.rs"), "// a, edited\n");
    assert!(
        cx.update(|cx| workspace.read(cx).agent_entries())
            .contains(&ChatEntry::Note(
                "Edits: accepted for the rest of this conversation.".into()
            ))
    );

    // The next one, in a file that isn't open, is written without a review
    // and without leaving the tab on screen.
    let next = write("e2", "b.rs", "// b, edited\n");
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.test_agent_request(next, window, cx))
    });
    assert_eq!(waiting(cx), None);
    assert_eq!(read("b.rs"), "// b, edited\n");
    assert_eq!(
        cx.update(|cx| workspace.read(cx).agent_entries()).last(),
        Some(&ChatEntry::Edit {
            path: "b.rs".into(),
            accepted: true,
            diff: vec![
                jig_commands::DiffLine::Removed("// b.rs".into()),
                jig_commands::DiffLine::Added("// b, edited".into()),
            ],
        })
    );
    assert!(
        cx.update(|cx| workspace.read(cx).editor().text(cx)) == "// a, edited\n",
        "still on a.rs"
    );

    // One in the open tab catches it up with the disk.
    let again = write("e3", "a.rs", "// a, edited twice\n");
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.test_agent_request(again, window, cx))
    });
    assert_eq!(waiting(cx), None);
    assert_eq!(
        cx.update(|cx| workspace.read(cx).editor().text(cx)),
        "// a, edited twice\n"
    );
}

#[gpui_kit::test]
fn an_edit_is_settled_with_its_buttons(cx: &mut TestAppContext) {
    use jig_ai::agent::{AgentEvent, EditChange, EditRequest};
    let (dir, window, workspace) = three_files(cx);
    let write = |id: &str, name: &str, text: &str| {
        AgentEvent::Edit(EditRequest {
            id: id.into(),
            path: dir.path().join(name),
            change: EditChange::Write(text.into()),
        })
    };
    let read = |name: &str| std::fs::read_to_string(dir.path().join(name)).unwrap();
    let waiting = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).agent_waiting_on());
    let click = |cx: &mut TestAppContext, button: &'static str| {
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| window.render_frame(cx));
        let at = vcx.debug_bounds(button).expect("the button shows");
        vcx.simulate_click(at.center(), gpui_kit::Modifiers::none());
        vcx.run_until_parked();
    };

    // Reject turns it down and leaves the file as it was.
    let first = write("e1", "a.rs", "// a, rejected\n");
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            this.test_agent_request(first, window, cx);
        })
    });
    assert_eq!(waiting(cx), Some("edit"));
    click(cx, "agent-decide-2");
    assert_eq!(waiting(cx), None);
    assert_eq!(read("a.rs"), "// a.rs\n");

    // Accept takes this one.
    let second = write("e2", "a.rs", "// a, accepted\n");
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.test_agent_request(second, window, cx))
    });
    click(cx, "agent-decide-0");
    assert_eq!(waiting(cx), None);
    assert_eq!(read("a.rs"), "// a, accepted\n");

    // Accept all takes this one and the next, unseen.
    let third = write("e3", "b.rs", "// b, accepted\n");
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.test_agent_request(third, window, cx))
    });
    assert_eq!(waiting(cx), Some("edit"));
    click(cx, "agent-decide-1");
    assert_eq!(read("b.rs"), "// b, accepted\n");
    let fourth = write("e4", "c.rs", "// c, accepted\n");
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.test_agent_request(fourth, window, cx))
    });
    assert_eq!(waiting(cx), None);
    assert_eq!(read("c.rs"), "// c, accepted\n");
}

#[gpui_kit::test]
fn a_long_command_scrolls_and_leaves_its_buttons_in_reach(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let command = (1..=200)
        .map(|n| format!("touch line{n}"))
        .collect::<Vec<_>>()
        .join(" && \n");
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            this.test_agent_request(permission(&command), window, cx);
        })
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let detail = vcx
        .debug_bounds("agent-request-detail")
        .expect("the command shows");
    assert!(detail.size.height <= px(141.), "{detail:?}");
    let height = vcx.update(|window, _| window.viewport_size().height);
    let allow = vcx.debug_bounds("agent-decide-0").expect("Allow shows");
    assert!(allow.bottom() < height, "{allow:?} in {height:?}");
}

#[gpui_kit::test]
fn escape_stops_the_agent_only_from_the_code_with_nothing_else_to_close(cx: &mut TestAppContext) {
    use jig_ai::agent::AgentEvent;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "counter = 1\n").unwrap();
    let (window, workspace) = open(cx, &path);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            this.test_agent_request(AgentEvent::Step("Reading".into()), window, cx);
            this.editor().select(12..12, cx);
            this.editor().focus(window, cx);
        })
    });
    let stopped = |cx: &mut TestAppContext| {
        cx.update(|cx| workspace.read(cx).agent_entries())
            .contains(&ChatEntry::Note("Stopped.".into()))
    };

    // Esc closes the completion list, and leaves the agent working.
    type_slowly(cx, window, "c");
    assert!(cx.update(|cx| {
        workspace
            .read(cx)
            .editor()
            .state()
            .read(cx)
            .completion_menu_state()
            .open
    }));
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    });
    assert!(!stopped(cx), "the agent works on");

    // Esc in the Tasks view leaves the code with the keyboard, and the agent
    // working.
    step(cx, window, |window, cx| window.press("ctrl-shift-t", cx));
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    });
    assert!(!stopped(cx), "the agent works on");
    assert!(cx.update(|cx| workspace.read(cx).chat.is_some()));

    // From the code, with nothing else open, it stops the agent.
    step(cx, window, |window, cx| window.press("escape", cx));
    assert!(stopped(cx));
}

#[gpui_kit::test]
fn edits_asked_for_at_once_are_reviewed_one_after_another(cx: &mut TestAppContext) {
    use jig_ai::agent::{AgentEvent, EditChange, EditRequest};
    let (dir, window, workspace) = three_files(cx);
    let write = |id: &str, name: &str, text: &str| {
        AgentEvent::Edit(EditRequest {
            id: id.into(),
            path: dir.path().join(name),
            change: EditChange::Write(text.into()),
        })
    };
    // The agent asks for both, then waits for both answers.
    let (first, second) = (
        write("e1", "a.rs", "// a, edited\n"),
        write("e2", "b.rs", "// b, edited\n"),
    );
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            this.test_agent_request(first, window, cx);
            this.test_agent_request(second, window, cx);
        })
    });
    let waiting = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).agent_waiting_on());
    let read = |name: &str| std::fs::read_to_string(dir.path().join(name)).unwrap();

    step(cx, window, |window, cx| window.press("tab", cx));
    assert_eq!(read("a.rs"), "// a, edited\n");
    assert_eq!(
        waiting(cx),
        Some("edit"),
        "the second shows at once, with nothing more from the agent"
    );
    step(cx, window, |window, cx| window.press("tab", cx));
    assert_eq!(read("b.rs"), "// b, edited\n");
    assert_eq!(waiting(cx), None);
}

#[gpui_kit::test]
fn an_edit_in_the_conversation_opens_to_its_diff(cx: &mut TestAppContext) {
    use jig_ai::agent::{AgentEvent, EditChange, EditRequest};
    let (dir, window, workspace) = three_files(cx);
    let edit = AgentEvent::Edit(EditRequest {
        id: "e1".into(),
        path: dir.path().join("a.rs"),
        change: EditChange::Write("// a, edited\n".into()),
    });
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            this.test_agent_request(edit, window, cx);
        })
    });
    step(cx, window, |window, cx| window.press("tab", cx));
    // "Hello", then the edit.
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(
        vcx.debug_bounds("agent-diff-1").is_none(),
        "folded at first"
    );
    let row = vcx.debug_bounds("agent-edit-1").expect("the edit's row");
    vcx.simulate_click(row.center(), gpui_kit::Modifiers::none());
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("agent-diff-1").is_some(), "open");
}
