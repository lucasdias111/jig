//! What the agent asks: commands to run, questions, plans to save as tasks, signing in.

use super::*;

pub(super) fn permission(detail: &str) -> jig_ai::agent::AgentEvent {
    jig_ai::agent::AgentEvent::Permission(jig_ai::agent::PermissionRequest {
        id: "per_1".into(),
        title: "Run a command".into(),
        detail: detail.into(),
        kind: jig_ai::agent::Asking::Command,
        always: None,
        task: None,
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
            let mut asking = permission("cargo fmt");
            if let jig_ai::agent::AgentEvent::Permission(request) = &mut asking {
                request.title = "Use the web".into();
                request.always = Some("Always allow `cargo fmt`".into());
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
        Some(&ChatEntry::Note("Always allow `cargo fmt`.".into()))
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

fn save_task() -> jig_ai::agent::AgentEvent {
    jig_ai::agent::AgentEvent::Permission(jig_ai::agent::PermissionRequest {
        id: "per_task".into(),
        title: "Save this plan as a task".into(),
        detail: String::new(),
        kind: jig_ai::agent::Asking::Other,
        always: Some("Always allow".into()),
        task: Some(jig_ai::agent::TaskDraft {
            title: "Split the parser".into(),
            todos: vec!["Move the lexer".into(), "Add tests".into()],
        }),
    })
}

#[gpui_kit::test]
fn saving_a_plan_is_a_question_answered_yes_no_or_in_words(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ask = |cx: &mut TestAppContext| {
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| {
                this.test_agent_request(save_task(), window, cx)
            })
        });
    };
    let waiting = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).agent_waiting_on());
    let last = |cx: &mut TestAppContext| {
        cx.update(|cx| workspace.read(cx).agent_entries().last().cloned())
    };
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.open_test_conversation(window, cx))
    });
    ask(cx);
    // The plan is shown with Save and Don't save, and no Always allow.
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("agent-task-draft").is_some());
    assert!(vcx.debug_bounds("agent-decide-0").is_some());
    assert!(vcx.debug_bounds("agent-decide-1").is_none());
    assert!(vcx.debug_bounds("agent-decide-2").is_some());

    // Enter with nothing typed saves it.
    let before = last(cx);
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(waiting(cx), None);
    assert_eq!(last(cx), before, "nothing to say about it");

    // Words in the reply box turn it down, for the agent to follow.
    ask(cx);
    step(cx, window, |window, cx| {
        window.input("leave out the tests", cx)
    });
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(waiting(cx), None);
    assert_eq!(
        last(cx),
        Some(ChatEntry::User("leave out the tests".into()))
    );

    // Esc doesn't save it.
    ask(cx);
    step(cx, window, |window, cx| window.press("escape", cx));
    assert_eq!(waiting(cx), None);
    assert_eq!(
        last(cx),
        Some(ChatEntry::Note("Not saved as a task.".into()))
    );

    // ⌘↩ just saves it: the next plan is asked about again.
    ask(cx);
    let before = last(cx);
    step(cx, window, |window, cx| window.press("secondary-enter", cx));
    assert_eq!(waiting(cx), None);
    assert_eq!(last(cx), before);
    ask(cx);
    assert_eq!(waiting(cx), Some("permission"));
}

#[gpui_kit::test]
fn an_agent_that_wants_a_sign_in_offers_its_ways(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let needed = jig_ai::agent::acp::SignInNeeded {
        agent: "Claude Code".into(),
        methods: vec![jig_ai::agent::acp::SignIn {
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
