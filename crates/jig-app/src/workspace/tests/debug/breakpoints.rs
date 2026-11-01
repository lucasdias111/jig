//! Breakpoints in the gutter, the debug button, and languages whose debugging is off.

use super::*;

#[gpui_kit::test]
fn clicking_the_gutter_toggles_a_breakpoint(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            let editor = this.editor().clone();
            editor.apply_edit(0..0, "one\ntwo\nthree\nfour\n", window, cx);
        });
    });
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let (bounds, line_height) = vcx.update(|_, cx| {
        let state = workspace.read(cx).editor().state().read(cx);
        (state.input_bounds(), state.line_height().unwrap())
    });
    // The line numbers start at the editor's left edge.
    let third_line = gpui_kit::point(
        bounds.origin.x + gpui_kit::px(6.),
        bounds.origin.y + line_height * 2.5,
    );
    let lines = |vcx: &mut gpui_kit::VisualTestContext| {
        vcx.update(|_, cx| {
            let this = workspace.read(cx);
            this.breakpoint_lines(this.active, cx)
        })
    };
    let selection = |vcx: &mut gpui_kit::VisualTestContext| {
        vcx.update(|_, cx| workspace.read(cx).editor().selection(cx))
    };
    let before = selection(&mut vcx);
    vcx.simulate_click(third_line, gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(lines(&mut vcx), [2].into());
    assert_eq!(
        selection(&mut vcx),
        before,
        "the click doesn't move the cursor"
    );
    vcx.update(|window, cx| window.render_frame(cx));
    vcx.simulate_click(third_line, gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert!(lines(&mut vcx).is_empty());
}

#[gpui_kit::test]
fn the_title_bar_debug_button_opens_the_picker(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = three_files(cx);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let button = vcx
        .debug_bounds("debug-selected")
        .expect("the button is in the title bar");
    vcx.simulate_click(button.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| workspace.read(cx).run_picker_rows(cx).is_some()));
}

#[gpui_kit::test]
fn breakpoints_follow_the_debugging_setting(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.ts");
    std::fs::write(&path, "const a = 1;\nconst b = 2;\n").unwrap();
    let (window, workspace) = open(cx, &path);
    let lines = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let this = workspace.read(cx);
            this.breakpoint_lines(this.active, cx)
        })
    };
    let toggle = |cx: &mut TestAppContext| {
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-f8", cx);
        })
    };

    // TypeScript's debugger is off to begin with: no breakpoints.
    toggle(cx);
    assert!(lines(cx).is_empty());

    step(cx, window, |_, cx| {
        jig_settings::update(cx, |s| s.debugging.set_enabled("typescript", true))
    });
    toggle(cx);
    assert_eq!(lines(cx), [0].into());

    // Off again hides it, and on again brings it back.
    step(cx, window, |_, cx| {
        jig_settings::update(cx, |s| s.debugging.set_enabled("typescript", false))
    });
    assert!(lines(cx).is_empty());
    step(cx, window, |_, cx| {
        jig_settings::update(cx, |s| s.debugging.set_enabled("typescript", true))
    });
    assert_eq!(lines(cx), [0].into());
}

#[gpui_kit::test]
fn debugging_a_language_that_is_off_starts_nothing(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".jig")).unwrap();
    std::fs::write(dir.path().join("app.ts"), "console.log(1);\n").unwrap();
    std::fs::write(
        dir.path().join(".jig/run.toml"),
        "[[run]]\nname = \"App\"\ncommand = \"node app.ts\"\n",
    )
    .unwrap();
    let (window, workspace) = open(cx, dir.path());
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("ctrl-d", cx);
    });
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).run_picker_rows(cx))
            .is_some_and(|rows| rows.first().is_some_and(|row| row == "App"))
    });
    step(cx, window, |window, cx| window.press("enter", cx));
    assert!(cx.update(|cx| workspace.read(cx).run_output(cx)).is_none());
}
