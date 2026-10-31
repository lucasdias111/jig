//! The modes: Code, the Planner and the Map in the editor's place, each
//! keeping its place while another shows, and the ways back to the code.

use super::*;
use crate::workspace::mode::Mode;

/// A project folder with `src/a.rs` open, the cursor on its third line.
fn project(cx: &mut TestAppContext) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/a.rs"),
        "fn a() {}\n\nfn b() {}\n\nfn c() {}\n",
    )
    .unwrap();
    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &dir.path().join("src/a.rs"));
    let at = "fn a() {}\n\n".len();
    cx.update(|cx| workspace.read(cx).editor().clone().select(at..at, cx));
    (dir, window, workspace)
}

fn mode(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Mode {
    cx.update(|cx| workspace.read(cx).mode)
}

fn set_mode(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    mode: Mode,
) {
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.set_mode(mode, window, cx));
        window.render_frame(cx);
    });
}

fn code_focused(cx: &mut TestAppContext, window: AnyWindowHandle, ws: &Entity<Workspace>) -> bool {
    let ws = ws.clone();
    cx.update_window(window, move |_, window, cx| {
        ws.read(cx)
            .editor()
            .state()
            .focus_handle(cx)
            .is_focused(window)
    })
    .unwrap()
}

#[gpui_kit::test]
fn the_planner_takes_the_codes_place_and_esc_goes_back(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = project(cx);
    set_mode(cx, window, &workspace, Mode::Planner);
    assert_eq!(mode(cx, &workspace), Mode::Planner);
    assert!(
        !code_focused(cx, window, &workspace),
        "the checklist has it"
    );
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("planner-mode").is_some());

    step(cx, window, |window, cx| window.press("escape", cx));
    assert_eq!(mode(cx, &workspace), Mode::Code);
    assert!(code_focused(cx, window, &workspace));
    assert_eq!(
        cx.update(|cx| workspace.read(cx).editor().cursor(cx)),
        "fn a() {}\n\n".len(),
        "the code kept its place"
    );
}

#[gpui_kit::test]
fn go_to_line_from_the_planner_lands_in_the_code(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = project(cx);
    set_mode(cx, window, &workspace, Mode::Planner);
    step(cx, window, |window, cx| {
        window.dispatch_action(Box::new(crate::workspace::GoToLine), cx)
    });
    step(cx, window, |window, cx| window.input("5", cx));
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(mode(cx, &workspace), Mode::Code);
    assert!(code_focused(cx, window, &workspace));
    assert_eq!(
        cx.update(|cx| workspace.read(cx).editor().cursor(cx)),
        "fn a() {}\n\nfn b() {}\n\n".len()
    );
}

#[gpui_kit::test]
fn another_tab_shows_the_code_and_go_back_returns_to_the_planner(cx: &mut TestAppContext) {
    let (dir, window, workspace) = project(cx);
    std::fs::write(dir.path().join("src/b.rs"), "fn b() {}\n").unwrap();
    set_mode(cx, window, &workspace, Mode::Planner);
    // As a task's link does: the place is remembered, then the code opens.
    let ws = workspace.clone();
    let b = dir.path().join("src/b.rs");
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.remember_location(cx);
            this.open_file(&b, window, cx);
        })
    });
    assert_eq!(mode(cx, &workspace), Mode::Code);
    step(cx, window, |window, cx| {
        window.dispatch_action(Box::new(crate::workspace::GoBack), cx)
    });
    assert_eq!(mode(cx, &workspace), Mode::Planner, "back to the Planner");
    step(cx, window, |window, cx| {
        window.dispatch_action(Box::new(crate::workspace::GoForward), cx)
    });
    assert_eq!(mode(cx, &workspace), Mode::Code);
    assert!(cx.update(|cx| workspace.read(cx).showing(&dir.path().join("src/b.rs"))));
}

#[gpui_kit::test]
fn the_map_is_a_mode_too(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = project(cx);
    set_mode(cx, window, &workspace, Mode::Map);
    assert_eq!(mode(cx, &workspace), Mode::Map);
    assert!(cx.update(|cx| workspace.read(cx).map_showing().is_some()));
    set_mode(cx, window, &workspace, Mode::Planner);
    assert_eq!(mode(cx, &workspace), Mode::Planner);
    assert!(cx.update(|cx| workspace.read(cx).map_showing().is_none()));
    set_mode(cx, window, &workspace, Mode::Code);
    assert!(code_focused(cx, window, &workspace));
}

fn press(cx: &mut TestAppContext, window: AnyWindowHandle, keys: &'static str) {
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        window.press(keys, cx);
    });
}

#[gpui_kit::test]
fn the_mode_keys_switch_modes(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = project(cx);
    press(cx, window, "secondary-2");
    assert_eq!(mode(cx, &workspace), Mode::Planner);
    press(cx, window, "secondary-3");
    assert_eq!(mode(cx, &workspace), Mode::Map);
    press(cx, window, "secondary-3");
    assert_eq!(mode(cx, &workspace), Mode::Map, "it stays");
    press(cx, window, "secondary-1");
    assert_eq!(mode(cx, &workspace), Mode::Code);
    assert!(code_focused(cx, window, &workspace));
    assert_eq!(
        cx.update(|cx| workspace.read(cx).editor().cursor(cx)),
        "fn a() {}\n\n".len(),
        "the code kept its place"
    );
    // ⌘B goes to the Map and back.
    press(cx, window, "secondary-b");
    assert_eq!(mode(cx, &workspace), Mode::Map);
    press(cx, window, "secondary-b");
    assert_eq!(mode(cx, &workspace), Mode::Code);
}

#[gpui_kit::test]
fn the_title_bar_switch_picks_a_mode_and_points_out_a_new_task(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = project(cx);
    let click = |cx: &mut TestAppContext, selector: &'static str| {
        step(cx, window, |window, cx| window.render_frame(cx));
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        let at = vcx.debug_bounds(selector).expect(selector).center();
        vcx.simulate_click(at, gpui_kit::Modifiers::none());
    };
    click(cx, "mode-planner");
    assert_eq!(mode(cx, &workspace), Mode::Planner);
    click(cx, "mode-code");
    assert_eq!(mode(cx, &workspace), Mode::Code);

    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("mode-planner-dot").is_none());
    cx.update(|cx| workspace.update(cx, |this, _| this.planner.unseen = true));
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("mode-planner-dot").is_some());
}

#[gpui_kit::test]
fn agent_mode_keeps_the_conversation_and_multiline_draft(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = project(cx);
    press(cx, window, "secondary-4");
    step(cx, window, |window, cx| {
        workspace.update(cx, |this, cx| this.new_agent_conversation(window, cx))
    });
    assert_eq!(mode(cx, &workspace), Mode::Agent);
    let id = cx.update(|cx| workspace.read(cx).chat.as_ref().unwrap().id);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let page = vcx.debug_bounds("agent-mode").expect("agent page");
    let reply = vcx.debug_bounds("agent-reply-box").expect("composer");
    assert!(page.size.width > px(700.), "uses the whole workspace");
    assert!(
        reply.origin.y > page.origin.y + page.size.height / 2.,
        "composer at the bottom"
    );
    assert!(vcx.debug_bounds("activity-files").is_none());
    assert!(vcx.debug_bounds("agent-chat-pin").is_none());
    assert!(vcx.debug_bounds("agent-chat-resize").is_none());
    step(cx, window, |window, cx| window.input("First line", cx));
    press(cx, window, "shift-enter");
    step(cx, window, |window, cx| window.input("Second line", cx));
    assert_eq!(
        cx.update(|cx| workspace.read(cx).agent_reply(cx).0),
        "First line\nSecond line"
    );
    assert!(
        cx.update(|cx| workspace.read(cx).agent_entries().is_empty()),
        "typing hasn't started a turn"
    );
    press(cx, window, "secondary-1");
    press(cx, window, "secondary-4");
    assert_eq!(
        cx.update(|cx| workspace.read(cx).chat.as_ref().unwrap().id),
        id
    );
    assert_eq!(
        cx.update(|cx| workspace.read(cx).agent_reply(cx).0),
        "First line\nSecond line"
    );
    press(cx, window, "escape");
    assert_eq!(mode(cx, &workspace), Mode::Agent);
    assert_eq!(
        cx.update(|cx| workspace.read(cx).chat.as_ref().unwrap().id),
        id,
        "Esc doesn't discard a conversation"
    );
}

#[gpui_kit::test]
fn agent_mode_works_before_opening_a_file_and_keeps_permissions_on_the_page(
    cx: &mut TestAppContext,
) {
    let dir = tempfile::tempdir().unwrap();
    let (window, workspace) = open(cx, dir.path());
    press(cx, window, "secondary-4");
    step(cx, window, |window, cx| {
        workspace.update(cx, |this, cx| this.new_agent_conversation(window, cx))
    });
    assert_eq!(mode(cx, &workspace), Mode::Agent);
    step(cx, window, |window, cx| {
        workspace.update(cx, |this, cx| {
            this.test_agent_request(
                jig_ai::agent::AgentEvent::Permission(jig_ai::agent::PermissionRequest {
                    id: "permission".into(),
                    title: "Run a command".into(),
                    detail: "cargo test".into(),
                    kind: jig_ai::agent::Asking::Command,
                    always: None,
                    task: None,
                }),
                window,
                cx,
            );
        })
    });
    assert_eq!(
        mode(cx, &workspace),
        Mode::Agent,
        "request stays on the page"
    );
    assert!(cx.update(|cx| workspace.read(cx).agent_waiting_on().is_some()));
    press(cx, window, "escape");
    assert_eq!(mode(cx, &workspace), Mode::Agent);
    assert!(cx.update(|cx| workspace.read(cx).agent_waiting_on().is_none()));
    press(cx, window, "secondary-k");
    assert!(
        cx.update(|cx| workspace.read(cx).palette.is_some()),
        "history and commands without a file"
    );
    press(cx, window, "escape");
    assert_eq!(mode(cx, &workspace), Mode::Agent);
}

#[gpui_kit::test]
fn edits_from_agent_mode_open_code_for_review_and_can_return(cx: &mut TestAppContext) {
    let (dir, window, workspace) = project(cx);
    press(cx, window, "secondary-4");
    step(cx, window, |window, cx| {
        workspace.update(cx, |this, cx| this.new_agent_conversation(window, cx))
    });
    let path = dir.path().join("src/a.rs");
    let before = std::fs::read_to_string(&path).unwrap();
    step(cx, window, |window, cx| {
        workspace.update(cx, |this, cx| {
            this.test_agent_request(
                jig_ai::agent::AgentEvent::Edit(jig_ai::agent::EditRequest {
                    id: "edit".into(),
                    path: path.clone(),
                    change: jig_ai::agent::EditChange::Write("fn changed() {}\n".into()),
                }),
                window,
                cx,
            );
        })
    });
    assert_eq!(mode(cx, &workspace), Mode::Code);
    assert!(cx.update(|cx| workspace.read(cx).previewing()));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "still needs approval"
    );
    press(cx, window, "secondary-4");
    step(cx, window, |window, cx| {
        workspace.update(cx, |this, cx| {
            assert!(this.show_agent_edit(window, cx));
        })
    });
    assert_eq!(
        mode(cx, &workspace),
        Mode::Code,
        "review opens even for the current file"
    );
    press(cx, window, "escape");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    press(cx, window, "secondary-4");
    assert_eq!(mode(cx, &workspace), Mode::Agent);
}

#[gpui_kit::test]
fn agent_mode_sends_a_project_message_without_hidden_cursor_context(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = project(cx);
    press(cx, window, "secondary-4");
    step(cx, window, |window, cx| {
        workspace.update(cx, |this, cx| this.new_agent_conversation(window, cx))
    });
    step(cx, window, |window, cx| {
        window.input("Explain this project", cx)
    });
    press(cx, window, "enter");
    assert_eq!(mode(cx, &workspace), Mode::Agent);
    let prompt = cx
        .update(|cx| workspace.read(cx).agent_prompt())
        .expect("sent the message");
    assert_eq!(jig_ai::agent::asked(&prompt), "Explain this project");
    assert!(
        !prompt.contains("fn b()"),
        "no implicit selection from the hidden editor"
    );
    assert_eq!(cx.update(|cx| workspace.read(cx).agent_reply(cx).0), "");
}

#[gpui_kit::test]
fn agent_home_lists_recent_conversations_and_opens_the_selected_one(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = project(cx);
    press(cx, window, "secondary-4");
    assert!(cx.update(|cx| workspace.read(cx).chat.is_none()));
    step(cx, window, |_, cx| {
        workspace.update(cx, |this, cx| {
            let now = std::time::SystemTime::now();
            this.test_recent_conversations(
                vec![
                    jig_ai::agent::Conversation {
                        id: "older".into(),
                        title: Some("Older conversation".into()),
                        updated: now - std::time::Duration::from_secs(3600),
                    },
                    jig_ai::agent::Conversation {
                        id: "newest".into(),
                        title: Some("Newest conversation".into()),
                        updated: now,
                    },
                ],
                cx,
            );
        })
    });
    step(cx, window, |window, cx| {
        window.input("Unsent new conversation", cx)
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("agent-recents").is_some());
    assert!(vcx.debug_bounds("agent-reply-box").is_some());
    let row = vcx
        .debug_bounds("agent-recent-0")
        .expect("recent conversation");
    vcx.simulate_click(row.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(
        vcx.update(|_, cx| workspace
            .read(cx)
            .chat
            .as_ref()
            .unwrap()
            .agent
            .as_ref()
            .unwrap()
            .label
            .clone()),
        "Newest conversation"
    );
    assert!(
        cx.update(|cx| workspace.read(cx).agent_prompt().is_none()),
        "choosing history does not send the home draft"
    );
    assert_eq!(mode(cx, &workspace), Mode::Agent);
}

#[gpui_kit::test]
fn agent_home_with_no_history_can_start_a_new_conversation(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = project(cx);
    press(cx, window, "secondary-4");
    step(cx, window, |_, cx| {
        workspace.update(cx, |this, cx| {
            this.test_recent_conversations(Vec::new(), cx)
        })
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("agent-recents").is_some());
    assert!(vcx.debug_bounds("agent-recent-0").is_none());
    assert!(vcx.debug_bounds("agent-reply-box").is_some());
    press(cx, window, "enter");
    assert!(
        cx.update(|cx| workspace.read(cx).chat.is_none()),
        "empty Enter starts nothing"
    );
    step(cx, window, |window, cx| {
        window.input("Explain this project", cx)
    });
    press(cx, window, "shift-enter");
    step(cx, window, |window, cx| {
        window.input("Start with its structure", cx)
    });
    assert!(
        cx.update(|cx| workspace.read(cx).chat.is_none()),
        "typing doesn't select a conversation"
    );
    press(cx, window, "secondary-1");
    press(cx, window, "secondary-4");
    press(cx, window, "enter");
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.debug_bounds("agent-recents").is_none());
    assert!(vcx.debug_bounds("agent-reply-box").is_some());
    let prompt = cx
        .update(|cx| workspace.read(cx).agent_prompt())
        .expect("sent a new message");
    assert_eq!(
        jig_ai::agent::asked(&prompt),
        "Explain this project\nStart with its structure"
    );
}
