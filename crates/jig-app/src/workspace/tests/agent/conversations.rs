//! The conversation itself: its project, model, long replies, folded tool calls, subagents, Esc, and code sent to it.

use super::*;

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

/// The agent conversation's status, which it keeps apart from ⌘K's bubble.
pub(super) fn chat_bubble(
    cx: &mut TestAppContext,
    workspace: &Entity<Workspace>,
) -> Option<Bubble> {
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
        jig_settings::update(cx, |s| s.agent.using = "claude-code".into());
        jig_agent::agents::remember_models(
            "claude-code",
            jig_ai::agent::acp::AgentModels {
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
fn command_k_offers_claude_codes_models(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    with_claude_code(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-k", cx);
    });
    let labels = cx.update(|cx| {
        let agent = workspace.read(cx).model_menu(cx);
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
    // Planner has the keyboard beside the docked conversation.
    vcx.update(|window, cx| window.press("ctrl-shift-t", cx));
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.update(|window, cx| workspace.read(cx).planner_focused(window, cx)));
    vcx.update(|window, cx| window.press("escape", cx));
    vcx.run_until_parked();
    assert!(
        vcx.update(|_, cx| workspace.read(cx).chat.is_none()),
        "Esc closes the idle conversation when leaving Planner"
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

#[gpui_kit::test]
fn a_subagents_work_has_a_row_of_its_own(cx: &mut TestAppContext) {
    use jig_ai::agent::AgentEvent;
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_test_conversation(window, cx);
            for event in [
                AgentEvent::Did("Ran `ls`".into()),
                AgentEvent::Subagent {
                    id: "s1".into(),
                    description: "Find the loader".into(),
                },
                AgentEvent::SubagentDid {
                    id: "s1".into(),
                    text: "Read src/config.rs".into(),
                },
                AgentEvent::SubagentDid {
                    id: "s1".into(),
                    text: "Ran `rg load`".into(),
                },
                AgentEvent::Did("Ran `cargo check`".into()),
                AgentEvent::SubagentDone {
                    id: "s1".into(),
                    report: "It's in config.rs.".into(),
                },
            ] {
                this.test_agent_request(event, window, cx);
            }
        })
    });
    let entries = cx.update(|cx| workspace.read(cx).agent_entries());
    assert_eq!(
        entries[1..],
        [
            ChatEntry::Did("Ran `ls`".into()),
            ChatEntry::Subagent {
                id: "s1".into(),
                description: "Find the loader".into(),
                calls: vec!["Read src/config.rs".into(), "Ran `rg load`".into()],
                report: Some("It's in config.rs.".into()),
            },
            ChatEntry::Did("Ran `cargo check`".into()),
        ],
        "its work under it, the agent's own lines apart"
    );

    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(
        vcx.debug_bounds("agent-subagent-work-2").is_none(),
        "folded"
    );
    let row = vcx
        .debug_bounds("agent-subagent-2")
        .expect("the subagent's row");
    vcx.simulate_click(row.center(), gpui_kit::Modifiers::none());
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("agent-subagent-work-2").is_some(), "open");
}

#[gpui_kit::test]
fn sending_code_opens_a_conversation_waiting_for_the_message(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn a() {}\nfn b() {}\nfn c() {}\n").unwrap();
    let (window, workspace) = open(cx, &path);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        ws.update(cx, |this, cx| this.editor().select(10..30, cx));
        window.press("secondary-shift-l", cx);
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(this.agent_entries().is_empty(), "nothing asked yet");
        let (text, references) = this.agent_reply(cx);
        assert_eq!(text, "@lib.rs:2-3 ");
        assert_eq!(references.len(), 1);
        assert_eq!(references[0].code, "fn b() {}\nfn c() {}\n");
        assert_eq!(this.agent_prompt(), None, "the agent isn't asked yet");
    });

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("why both?", cx);
    });
    step(cx, window, |window, cx| window.press("enter", cx));
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert_eq!(
            this.agent_entries().first(),
            Some(&ChatEntry::User("@lib.rs:2-3 why both?".into()))
        );
        let prompt = this.agent_prompt().expect("sent");
        assert!(
            prompt.contains("@lib.rs:2-3:\n```rust\nfn b() {}\nfn c() {}\n```"),
            "{prompt}"
        );
        assert_eq!(jig_ai::agent::asked(&prompt), "@lib.rs:2-3 why both?");
    });
}

#[gpui_kit::test]
fn sending_code_to_an_open_conversation_adds_it_to_the_reply(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn a() {}\nfn b() {}\n").unwrap();
    let (window, workspace) = open(cx, &path);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.open_test_conversation(window, cx))
    });
    let id = cx.update(|cx| workspace.read(cx).chat.as_ref().unwrap().id);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            let input = this
                .chat
                .as_ref()
                .unwrap()
                .agent
                .as_ref()
                .unwrap()
                .input
                .clone();
            input.update(cx, |input, cx| {
                input.set_value("look at", window, cx);
                input.set_cursor_position(lsp_types::Position::new(0, 7), window, cx);
            });
            this.focus_code(window, cx);
            // Nothing selected: the cursor's line.
            this.editor().select(3..3, cx);
        });
        window.press("secondary-shift-l", cx);
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert_eq!(this.chat.as_ref().unwrap().id, id, "the same conversation");
        let (text, references) = this.agent_reply(cx);
        assert_eq!(text, "look at @lib.rs:1 ");
        assert_eq!(references[0].code, "fn a() {}");
    });
}
