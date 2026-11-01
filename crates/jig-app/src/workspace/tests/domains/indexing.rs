//! Index Domains and its drafts, as a conversation using the Map's tools.

use super::*;

#[gpui_kit::test]
fn domain_shows_jigs_drafts_under_the_index_banner(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = open_project(cx, false);
    let view = to_domain(cx, window, &workspace);
    let (regions, banner) =
        cx.update(|cx| (view.read(cx).domain_regions(), view.read(cx).banner()));
    assert_eq!(
        banner.as_deref(),
        Some("Index Domains to name and describe these")
    );
    // The folders, by the words their functions use.
    assert!(regions.len() >= 3, "{regions:?}");
    assert!(
        regions.iter().any(|name| name.contains("booking")),
        "{regions:?}"
    );
    // Drafts can't be edited by hand.
    let event = cx.update(|cx| view.read(cx).region_point(&regions[0], false));
    click(cx, window, event);
    press(cx, window, "f2");
    assert!(cx.update(|cx| view.read(cx).domain_input()).is_none());
    // ⌘↓ goes into Site.
    press(cx, window, "secondary-down");
    assert_eq!(cx.update(|cx| view.read(cx).level().clone()), Level::Site);
}

#[gpui_kit::test]
fn index_domains_is_a_conversation_whose_answer_is_kept_on_the_map(cx: &mut TestAppContext) {
    let (dir, window, workspace) = open_project(cx, false);
    let view = to_domain(cx, window, &workspace);
    click_on(cx, window, "map-banner-index");
    let prompt = cx
        .update(|cx| workspace.read(cx).agent_prompt())
        .expect("a conversation");
    assert!(prompt.starts_with("You are indexing this project's business domains"));
    // Every source file, the page the index can't read too.
    assert!(prompt.contains("- web/App.svelte\n"), "{prompt}");
    assert!(prompt.contains("never routes"));
    // A command that only reads runs without asking; others ask.
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.test_agent_request(
                jig_ai::agent::AgentEvent::Permission(jig_ai::agent::PermissionRequest {
                    id: "per_1".into(),
                    title: "Run a command".into(),
                    detail: "rg -n book src | head".into(),
                    kind: jig_ai::agent::Asking::Command,
                    always: None,
                    task: None,
                }),
                window,
                cx,
            )
        })
    });
    assert_eq!(cx.update(|cx| workspace.read(cx).agent_waiting_on()), None);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.test_agent_request(
                jig_ai::agent::AgentEvent::Permission(jig_ai::agent::PermissionRequest {
                    id: "per_2".into(),
                    title: "Run a command".into(),
                    detail: "rm -rf target".into(),
                    kind: jig_ai::agent::Asking::Command,
                    always: None,
                    task: None,
                }),
                window,
                cx,
            )
        })
    });
    assert_eq!(
        cx.update(|cx| workspace.read(cx).agent_waiting_on()),
        Some("permission")
    );
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.settle_agent_request(false, window, cx));
    });
    answer(
        cx,
        window,
        &workspace,
        &format!("Here:\n\n```toml\n{SAVED}```\n"),
    );
    let view = map(cx, &workspace).unwrap_or(view);
    let (regions, banner) =
        cx.update(|cx| (view.read(cx).domain_regions(), view.read(cx).banner()));
    assert_eq!(banner.as_deref(), Some("The agent proposes these domains"));
    assert_eq!(regions, ["Bookings", "Events", "Shared"]);
    assert!(saved(&dir).is_empty(), "nothing saved before it's kept");
    // Kept: saved, and shown as the domains.
    press(cx, window, "secondary-enter");
    assert!(!cx.update(|cx| view.read(cx).has_proposal()));
    let text = saved(&dir);
    assert!(text.contains("name = \"Bookings\""), "{text}");
    assert!(
        text.contains("members = [\n  \"src/bookings/\"\n]"),
        "{text}"
    );
    assert_eq!(cx.update(|cx| view.read(cx).banner()), None);
    // ⌘Z takes back the keeping: the proposal again.
    press(cx, window, "secondary-z");
    assert!(cx.update(|cx| view.read(cx).has_proposal()));
    assert!(saved(&dir).is_empty());
}

#[gpui_kit::test]
fn index_domains_with_the_maps_tools_asks_for_nothing(cx: &mut TestAppContext) {
    use jig_ai::agent::{AgentEvent, Asking, PermissionRequest};
    let (dir, window, workspace) = open_project(cx, false);
    to_domain(cx, window, &workspace);
    click_on(cx, window, "map-banner-index");
    let prompt = cx.update(|cx| workspace.read(cx).agent_prompt()).unwrap();
    assert!(prompt.contains("map_project") && prompt.contains("map_propose_domains"));
    assert!(prompt.contains("Don't run shell commands and don't start subagents"));
    let send = |cx: &mut TestAppContext, event: AgentEvent| {
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| this.test_agent_request(event, window, cx))
        });
        assert_eq!(
            cx.update(|cx| workspace.read(cx).agent_waiting_on()),
            None,
            "nothing waits for the user"
        );
    };
    let request = |id: &str, title: &str, detail: &str, kind: Asking| {
        AgentEvent::Permission(PermissionRequest {
            id: id.into(),
            title: title.into(),
            detail: detail.into(),
            kind,
            always: None,
            task: None,
        })
    };
    send(
        cx,
        AgentEvent::Drew {
            tool: jig_ai::map::PROJECT.into(),
            input: serde_json::json!({"group": 1}),
        },
    );
    // Should it still reach for a shell or a subagent, what only reads
    // goes ahead, however the agent words it.
    send(
        cx,
        request(
            "per_1",
            "Bash",
            "ls src 2>/dev/null | head",
            Asking::Command,
        ),
    );
    send(
        cx,
        request(
            "per_2",
            "Start a subagent",
            "Find the entry points",
            Asking::Subagent { explores: true },
        ),
    );
    send(
        cx,
        AgentEvent::Drew {
            tool: jig_ai::map::PROPOSE_DOMAINS.into(),
            input: serde_json::json!({
                "action": "index",
                "domains": [
                    {"name": "Bookings", "purpose": "Book a place at an event.", "offers": ["Booking form"], "why": "They take bookings.", "members": ["src/bookings/"]},
                    {"name": "Events", "purpose": "List what's on.", "offers": ["Event list"], "why": "They show events.", "members": ["src/events/", "web/"]}
                ],
                "flows": [{"from": "Bookings", "to": "Events", "label": "asks for events"}],
                "shared": ["src/util.rs", "src/main.rs"]
            }),
        },
    );
    // Its closing words hold no TOML: the call is the answer.
    answer(cx, window, &workspace, "Proposed two domains on the Map.");
    let view = map(cx, &workspace).unwrap();
    assert_eq!(
        cx.update(|cx| view.read(cx).domain_regions()),
        ["Bookings", "Events", "Shared"]
    );
    press(cx, window, "secondary-enter");
    assert!(saved(&dir).contains("label = \"asks for events\""));
    // Outside a Map action, a command that only reads goes ahead too, and
    // others still ask.
    send_outside_a_map_action(cx, window, &workspace);
}

/// A conversation at the code: a command that only reads goes ahead, one
/// that builds asks.
fn send_outside_a_map_action(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
) {
    press(cx, window, "secondary-b");
    press(cx, window, "secondary-k");
    type_slowly(cx, window, "Explain this");
    press(cx, window, "enter");
    for (id, command, waits) in [
        ("per_3", "ls src 2>/dev/null", None),
        ("per_4", "cargo build", Some("permission")),
    ] {
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| {
                this.test_agent_request(
                    jig_ai::agent::AgentEvent::Permission(jig_ai::agent::PermissionRequest {
                        id: id.into(),
                        title: "Run a command".into(),
                        detail: command.into(),
                        kind: jig_ai::agent::Asking::Command,
                        always: None,
                        task: None,
                    }),
                    window,
                    cx,
                )
            })
        });
        assert_eq!(
            cx.update(|cx| workspace.read(cx).agent_waiting_on()),
            waits,
            "{command}"
        );
    }
}
