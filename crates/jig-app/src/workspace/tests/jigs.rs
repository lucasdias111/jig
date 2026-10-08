//! The command input, the add-jig form and the button beside a selection.

use super::*;

#[gpui_kit::test]
fn add_command_from_the_shortcut(cx: &mut TestAppContext) {
    let (_dir, window, workspace, commands) = open_with_commands(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-shift-k", cx);
    });
    assert!(cx.update(|cx| workspace.read(cx).new_command.is_some()));
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("Create controller", cx);
        window.press("tab", cx);
        window.input("Insert a REST controller at the cursor.", cx);
        window.press("secondary-2", cx);
        window.press("secondary-enter", cx);
    });

    let source = std::fs::read_to_string(&commands).unwrap();
    let saved = jig_commands::presets::parse(&source).unwrap();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].name, "Create controller");
    assert_eq!(saved[0].prompt, "Insert a REST controller at the cursor.");
    assert_eq!(saved[0].scope, jig_commands::Scope::Cursor);
    assert!(!saved[0].agent, "quick unless chosen otherwise");
    assert!(
        preset_names(cx, &workspace).contains(&"Create controller".to_string()),
        "available in ⌘K at once"
    );
    step(cx, window, |window, cx| {
        let this = workspace.read(cx);
        assert!(this.new_command.is_none());
        assert!(this.editor().state().focus_handle(cx).is_focused(window));
        assert!(
            matches!(this.run.as_ref().map(|r| &r.bubble), Some(Bubble::Message(m)) if m.contains("Create controller"))
        );
        assert_eq!(
            this.editor().text(cx),
            ORIGINAL,
            "the file being edited is untouched"
        );
    });
}

#[gpui_kit::test]
fn save_typed_instruction_as_command(cx: &mut TestAppContext) {
    let (_dir, window, workspace, commands) = open_with_commands(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-k", cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("wrap this in a tokio task", cx);
        window.press("secondary-enter", cx);
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(this.palette.is_none());
        assert!(
            this.new_command.is_some(),
            "the form replaces the command input"
        );
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("Spawn task", cx);
        window.press("secondary-enter", cx);
    });
    let saved = jig_commands::presets::parse(&std::fs::read_to_string(&commands).unwrap()).unwrap();
    assert_eq!(
        saved[0].prompt, "wrap this in a tokio task",
        "the typed text becomes the prompt"
    );
    assert_eq!(saved[0].scope, jig_commands::Scope::Selection);
}

#[gpui_kit::test]
fn duplicate_name_keeps_the_form_open(cx: &mut TestAppContext) {
    let (_dir, window, workspace, commands) = open_with_commands(cx);
    std::fs::create_dir_all(commands.parent().unwrap()).unwrap();
    std::fs::write(&commands, "[[jig]]\nname = \"Mine\"\nprompt = \"p\"\n").unwrap();
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-shift-k", cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("mine", cx);
        window.press("tab", cx);
        window.input("again", cx);
        window.press("secondary-enter", cx);
    });
    assert!(cx.update(|cx| workspace.read(cx).new_command.is_some()));
    assert_eq!(
        jig_commands::presets::parse(&std::fs::read_to_string(&commands).unwrap())
            .unwrap()
            .len(),
        1
    );
}

#[gpui_kit::test]
fn escape_cancels_the_form(cx: &mut TestAppContext) {
    let (_dir, window, workspace, commands) = open_with_commands(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-shift-k", cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("Half typed", cx);
        window.press("escape", cx);
    });
    assert!(cx.update(|cx| workspace.read(cx).new_command.is_none()));
    assert!(!commands.exists());
}

#[gpui_kit::test]
fn the_title_bar_button_opens_the_command_input(cx: &mut TestAppContext) {
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
fn a_button_beside_the_selection_opens_the_command_input(cx: &mut TestAppContext) {
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
        "hidden while the command input is open"
    );
    assert_eq!(
        vcx.update(|_, cx| workspace.read(cx).editor().selection(cx)),
        0..7,
        "the click kept the selection"
    );
}

#[gpui_kit::test]
fn clicks_on_the_form_stay_in_the_form(cx: &mut TestAppContext) {
    let (_dir, window, workspace, _) = open_with_commands(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-shift-k", cx);
    });
    // On the form's title, away from its inputs, over the code. The
    // form is 460 wide, centred, just under the 46px title bar.
    step(cx, window, |window, cx| {
        let title = gpui_kit::point(px(400.), px(74.));
        window.drag(title, title, cx);
    });
    step(cx, window, |window, cx| {
        let this = workspace.read(cx);
        assert!(
            !this.editor().state().focus_handle(cx).is_focused(window),
            "the editor behind didn't take the click"
        );
        window.press("escape", cx);
    });
    assert!(cx.update(|cx| workspace.read(cx).new_command.is_none()));
}

#[gpui_kit::test]
fn escape_closes_the_form_after_clicking_the_code(cx: &mut TestAppContext) {
    let (_dir, window, workspace, _) = open_with_commands(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-shift-k", cx);
    });
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        let editor = ws.read(cx).editor().state().clone();
        editor.update(cx, |editor, cx| editor.focus(window, cx));
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    });
    assert!(cx.update(|cx| workspace.read(cx).new_command.is_none()));
}

#[gpui_kit::test]
fn saving_the_commands_file_reloads_presets(cx: &mut TestAppContext) {
    let (_dir, window, workspace, commands) = open_with_commands(cx);
    cx.update(|cx| {
        cx.update_window(window, |_, window, cx| {
            workspace.update(cx, |this, cx| {
                this.edit_commands(&crate::workspace::EditCommands, window, cx)
            })
        })
        .unwrap()
    });
    cx.run_until_parked();
    assert!(commands.exists(), "created with a header");
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            let end = this.editor().text(cx).len();
            this.editor().apply_edit(
                end..end,
                "\n[[jig]]\nname = \"From file\"\nprompt = \"p\"\n",
                window,
                cx,
            );
        });
        window.press("secondary-s", cx);
    });
    assert!(preset_names(cx, &workspace).contains(&"From file".to_string()));
}
