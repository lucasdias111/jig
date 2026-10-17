//! The agent input (⌘K), its title-bar button and the button beside a
//! selection.

use super::*;

#[gpui_kit::test]
fn the_title_bar_button_opens_the_agent_input(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let button = vcx
        .debug_bounds("open-command")
        .expect("the button is in the title bar");
    vcx.simulate_click(button.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| workspace.read(cx).palette.is_some()));
}

#[gpui_kit::test]
fn a_button_beside_the_selection_opens_the_agent_input(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(
        vcx.debug_bounds("selection-command").is_none(),
        "nothing selected, no button"
    );

    let ws = workspace.clone();
    vcx.update(|window, cx| {
        ws.update(cx, |this, cx| {
            this.editor().focus(window, cx);
            this.editor()
                .state()
                .update(cx, |s, cx| s.set_selected_range(0..7, cx))
        });
        window.render_frame(cx);
    });
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    let button = vcx
        .debug_bounds("selection-command")
        .expect("the button shows beside the selection");
    let line = vcx
        .update(|_, cx| workspace.read(cx).editor().beside_point(0..7, cx))
        .unwrap();
    assert!(button.left() >= line.x, "right of the selected text");

    // Settings can turn it off, and back on.
    vcx.update(|window, cx| {
        crate::settings::update(cx, |s| s.editor.selection_button = false);
        window.render_frame(cx);
    });
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(
        vcx.debug_bounds("selection-command").is_none(),
        "turned off in Settings"
    );
    vcx.update(|window, cx| {
        crate::settings::update(cx, |s| s.editor.selection_button = true);
        window.render_frame(cx);
    });
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    let button = vcx
        .debug_bounds("selection-command")
        .expect("back once turned on");

    vcx.simulate_click(button.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    assert!(vcx.update(|_, cx| workspace.read(cx).palette.is_some()));
    assert!(
        vcx.debug_bounds("selection-command").is_none(),
        "hidden while the input is open"
    );
    assert_eq!(
        vcx.update(|_, cx| workspace.read(cx).editor().selection(cx)),
        0..7,
        "the click kept the selection"
    );
}

#[gpui_kit::test]
fn command_k_and_the_agent_button_toggle_the_input(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let open = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).palette.is_some());
    press(cx, window, "secondary-k");
    assert!(open(cx));
    press(cx, window, "secondary-k");
    assert!(!open(cx), "⌘K again closes it");

    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let button = vcx.debug_bounds("open-command").expect("the Agent button");
    vcx.simulate_click(button.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| workspace.read(cx).palette.is_some()));
    vcx.update(|window, cx| window.render_frame(cx));
    vcx.simulate_click(button.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert!(
        vcx.update(|_, cx| workspace.read(cx).palette.is_none()),
        "clicking it again closes it, and doesn't open it anew"
    );
}

#[gpui_kit::test]
fn the_agent_input_opens_beside_the_code_when_it_fits(cx: &mut TestAppContext) {
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
                this.palette = None;
                this.editor().select(selected.clone(), cx);
                this.open_command(&super::super::OpenCommand, window, cx);
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
            (this.palette.as_ref().unwrap().anchor, end)
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
fn escape_closes_the_input(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, ORIGINAL).unwrap();
    let (window, workspace) = open(cx, &path);

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-k", cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    });
    step(cx, window, |window, cx| {
        let this = workspace.read(cx);
        assert!(this.palette.is_none());
        assert!(this.chat.is_none());
        assert!(this.editor().state().focus_handle(cx).is_focused(window));
    });
}

#[gpui_kit::test]
fn a_typed_message_starts_a_conversation_about_the_line(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn a() {}\nfn b() {}\n").unwrap();
    let (window, workspace) = open(cx, &path);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        ws.update(cx, |this, cx| this.editor().select(12..12, cx));
        window.press("secondary-k", cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("rename it", cx);
        window.press("enter", cx);
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(this.palette.is_none());
        let chat = this.chat.as_ref().expect("a conversation");
        assert_eq!(chat.target, 10..19, "the cursor's line");
    });
    assert_eq!(
        cx.update(|cx| workspace.read(cx).agent_entries()).first(),
        Some(&ChatEntry::User("rename it".into()))
    );
}

#[gpui_kit::test]
fn escape_rejects_the_agent_s_edit(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = preview_docs(cx);
    assert_eq!(text(cx, &workspace), DOCUMENTED);
    step(cx, window, |window, cx| window.press("escape", cx));
    assert_eq!(text(cx, &workspace), ORIGINAL);
    assert!(!cx.update(|cx| workspace.read(cx).previewing()));
}
