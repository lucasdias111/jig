//! Tabs, each with its own buffer, and closing them.

use super::*;

#[gpui_kit::test]
fn language_settings_rehighlight_open_tabs(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.ts");
    std::fs::write(&path, "const a = 1;\n").unwrap();
    let (window, workspace) = open(cx, &path);

    step(cx, window, |_, cx| {
        assert_eq!(workspace.read(cx).editor().language(cx), "typescript");
        crate::settings::update(cx, |s| s.languages.set_off("typescript", true));
    });
    step(cx, window, |_, cx| {
        assert_eq!(workspace.read(cx).editor().language(cx), "text");
        crate::settings::update(cx, |s| s.languages.set_off("typescript", false));
    });
    step(cx, window, |_, cx| {
        assert_eq!(workspace.read(cx).editor().language(cx), "typescript");
    });
}

fn tab_titles(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
    cx.update(|cx| {
        let this = workspace.read(cx);
        this.tabs.iter().map(|tab| tab.document.title()).collect()
    })
}

fn active_title(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> String {
    cx.update(|cx| workspace.read(cx).document().title())
}

#[gpui_kit::test]
fn each_tab_keeps_its_own_buffer_and_undo(cx: &mut TestAppContext) {
    let (dir, window, workspace) = three_files(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("x", cx);
    });
    open_file(cx, window, &workspace, &dir.path().join("b.rs"));
    assert_eq!(tab_titles(cx, &workspace), ["a.rs", "b.rs"]);
    assert_eq!(active_title(cx, &workspace), "b.rs");
    assert!(!cx.update(|cx| workspace.read(cx).tab().dirty));
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        assert!(
            workspace
                .read(cx)
                .editor()
                .state()
                .focus_handle(cx)
                .is_focused(window),
            "the new tab has the keyboard"
        );
        window.input("y", cx);
    });
    assert_eq!(text(cx, &workspace), "y// b.rs\n");

    // Back to a.rs: its edit is still there and still unsaved.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-1", cx);
    });
    assert_eq!(text(cx, &workspace), "x// a.rs\n");
    assert!(cx.update(|cx| workspace.read(cx).tab().dirty));

    // Undo in a.rs leaves b.rs alone.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-z", cx);
    });
    assert_eq!(text(cx, &workspace), "// a.rs\n");
    assert!(!cx.update(|cx| workspace.read(cx).tab().dirty));
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-shift-]", cx);
    });
    assert_eq!(text(cx, &workspace), "y// b.rs\n");
}

#[gpui_kit::test]
fn opening_an_open_file_switches_to_its_tab(cx: &mut TestAppContext) {
    let (dir, window, workspace) = three_files(cx);
    open_file(cx, window, &workspace, &dir.path().join("b.rs"));
    open_file(cx, window, &workspace, &dir.path().join("c.rs"));
    open_file(cx, window, &workspace, &dir.path().join("a.rs"));
    assert_eq!(tab_titles(cx, &workspace), ["a.rs", "b.rs", "c.rs"]);
    assert_eq!(active_title(cx, &workspace), "a.rs");

    // New tabs open next to the current one.
    std::fs::write(dir.path().join("d.rs"), "").unwrap();
    open_file(cx, window, &workspace, &dir.path().join("d.rs"));
    assert_eq!(tab_titles(cx, &workspace), ["a.rs", "d.rs", "b.rs", "c.rs"]);

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-9", cx);
    });
    assert_eq!(
        active_title(cx, &workspace),
        "c.rs",
        "Cmd+9 is the last tab"
    );
    step(cx, window, |window, cx| window.press("ctrl-tab", cx));
    assert_eq!(active_title(cx, &workspace), "a.rs", "wraps around");
}

#[gpui_kit::test]
fn a_file_from_elsewhere_keeps_the_project(cx: &mut TestAppContext) {
    let (dir, window, workspace) = three_files(cx);
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::write(elsewhere.path().join("debuggers.toml"), "").unwrap();
    let root = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).project_root(cx));
    let project = Some(dir.path().canonicalize().unwrap());

    open_file(
        cx,
        window,
        &workspace,
        &elsewhere.path().join("debuggers.toml"),
    );
    assert_eq!(active_title(cx, &workspace), "debuggers.toml");
    assert_eq!(root(cx), project);

    open_file(cx, window, &workspace, &dir.path().join("b.rs"));
    assert_eq!(root(cx), project);
}

#[gpui_kit::test]
fn closing_tabs(cx: &mut TestAppContext) {
    let (dir, window, workspace) = three_files(cx);
    open_file(cx, window, &workspace, &dir.path().join("b.rs"));
    open_file(cx, window, &workspace, &dir.path().join("c.rs"));
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-2", cx);
    });

    // A clean tab closes at once; its right neighbour takes over.
    step(cx, window, |window, cx| window.press("secondary-w", cx));
    assert_eq!(tab_titles(cx, &workspace), ["a.rs", "c.rs"]);
    assert_eq!(active_title(cx, &workspace), "c.rs");

    // An unsaved one asks first.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("z", cx);
    });
    step(cx, window, |window, cx| window.press("secondary-w", cx));
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(tab_titles(cx, &workspace), ["a.rs", "c.rs"]);
    step(cx, window, |window, cx| window.press("secondary-w", cx));
    cx.simulate_prompt_answer("Discard");
    cx.run_until_parked();
    assert_eq!(
        tab_titles(cx, &workspace),
        ["a.rs"],
        "the last tab closed goes left"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("c.rs")).unwrap(),
        "// c.rs\n"
    );

    // Closing the last file leaves an empty Untitled tab.
    step(cx, window, |window, cx| window.press("secondary-w", cx));
    assert_eq!(tab_titles(cx, &workspace), ["Untitled"]);
    assert_eq!(text(cx, &workspace), "");
}

#[gpui_kit::test]
fn closing_other_and_all_tabs(cx: &mut TestAppContext) {
    let (dir, window, workspace) = three_files(cx);
    open_file(cx, window, &workspace, &dir.path().join("b.rs"));
    open_file(cx, window, &workspace, &dir.path().join("c.rs"));
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-2", cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("z", cx);
    });
    let close_except = |cx: &mut TestAppContext, keep: Option<usize>| {
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| this.close_tabs_except(keep, window, cx))
        });
    };

    // Closing the others asks once about the unsaved b.rs.
    close_except(cx, Some(2));
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(tab_titles(cx, &workspace), ["a.rs", "b.rs", "c.rs"]);
    close_except(cx, Some(2));
    cx.simulate_prompt_answer("Discard");
    cx.run_until_parked();
    assert_eq!(tab_titles(cx, &workspace), ["c.rs"]);
    assert_eq!(active_title(cx, &workspace), "c.rs");

    // Closing them all shows the start page.
    open_file(cx, window, &workspace, &dir.path().join("a.rs"));
    close_except(cx, None);
    assert!(!cx.has_pending_prompt());
    assert!(cx.update(|cx| workspace.read(cx).home));
    assert_eq!(tab_titles(cx, &workspace), ["Untitled"]);
}

#[gpui_kit::test]
fn a_blank_tab_is_reused(cx: &mut TestAppContext) {
    let (dir, window, workspace) = three_files(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-n", cx);
    });
    assert_eq!(tab_titles(cx, &workspace), ["a.rs", "Untitled"]);
    open_file(cx, window, &workspace, &dir.path().join("b.rs"));
    assert_eq!(
        tab_titles(cx, &workspace),
        ["a.rs", "b.rs"],
        "the untouched Untitled tab became b.rs"
    );
}

#[gpui_kit::test]
fn switching_tabs_keeps_a_change_under_review(cx: &mut TestAppContext) {
    let (dir, window, workspace) = preview_docs(cx);
    std::fs::write(dir.path().join("b.rs"), "").unwrap();
    open_file(cx, window, &workspace, &dir.path().join("b.rs"));
    assert!(bubble(cx, &workspace).is_none());
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-1", cx);
    });
    assert_eq!(text(cx, &workspace), DOCUMENTED);
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(this.editor().state().read(cx).is_editable());
        assert!(this.editor().highlighted_ranges(cx).is_empty());
    });
}

#[gpui_kit::test]
fn same_named_files_show_their_folder(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    for folder in ["app", "core"] {
        std::fs::create_dir_all(dir.path().join(folder)).unwrap();
        std::fs::write(dir.path().join(folder).join("mod.rs"), "").unwrap();
    }
    std::fs::write(dir.path().join("main.rs"), "").unwrap();
    let (window, workspace) = open(cx, &dir.path().join("app/mod.rs"));
    open_file(cx, window, &workspace, &dir.path().join("core/mod.rs"));
    open_file(cx, window, &workspace, &dir.path().join("main.rs"));
    assert_eq!(
        cx.update(|cx| workspace.read(cx).tab_labels()),
        ["mod.rs — app", "mod.rs — core", "main.rs"]
    );
}

#[gpui_kit::test]
fn settings_changes_reach_open_tabs_and_commands(cx: &mut TestAppContext) {
    let (dir, window, workspace) = three_files(cx);
    assert!(preset_names(cx, &workspace).contains(&"Add docs".to_string()));

    cx.update(|cx| {
        crate::settings::update(cx, |s| {
            s.editor.soft_wrap = false;
            s.commands.set_hidden("add docs", true);
        })
    });
    cx.run_until_parked();
    assert!(
        !preset_names(cx, &workspace).contains(&"Add docs".to_string()),
        "hidden commands leave ⌘K"
    );
    // Tabs opened later start with the new settings too.
    open_file(cx, window, &workspace, &dir.path().join("b.rs"));
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(!this.settings.editor.soft_wrap);
        assert_eq!(this.tabs.len(), 2);
    });

    cx.update(|cx| crate::settings::update(cx, |s| s.commands.hidden.clear()));
    cx.run_until_parked();
    assert!(preset_names(cx, &workspace).contains(&"Add docs".to_string()));
}

#[gpui_kit::test]
fn closing_the_window_asks_about_every_unsaved_tab(cx: &mut TestAppContext) {
    let (dir, window, workspace) = three_files(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("x", cx);
    });
    open_file(cx, window, &workspace, &dir.path().join("b.rs"));
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-shift-w", cx);
    });
    assert!(
        cx.has_pending_prompt(),
        "a.rs is unsaved, though not in view"
    );
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(tab_titles(cx, &workspace), ["a.rs", "b.rs"]);
}
