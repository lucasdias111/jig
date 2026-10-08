//! The home page, project pages and where Settings sits.

use super::*;

/// A window as Jig opens with nothing to show.
fn open_empty(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
    cx.update(|cx| {
        gpui_kit::init(cx);
        cx.bind_keys(key_bindings());
    });
    let handles = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(None, window, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    handles
}

#[gpui_kit::test]
fn starts_on_the_home_page_and_new_file_leaves_it(cx: &mut TestAppContext) {
    let (window, workspace) = open_empty(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        let this = workspace.read(cx);
        assert!(this.home);
        assert!(this.home_focus.is_focused(window));
        window.press("secondary-k", cx);
    });
    assert!(
        cx.update(|cx| workspace.read(cx).palette.is_none()),
        "no commands without a file"
    );
    step(cx, window, |window, cx| window.press("secondary-n", cx));
    step(cx, window, |window, cx| {
        let this = workspace.read(cx);
        assert!(!this.home);
        assert_eq!(this.tabs.len(), 1, "the blank tab is the new file");
        assert!(this.editor().state().focus_handle(cx).is_focused(window));
    });
}

#[gpui_kit::test]
fn a_project_with_no_file_open_shows_its_page(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), ORIGINAL).unwrap();
    let (window, workspace) = open(cx, dir.path());
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(this.home, "no Untitled tab");
        assert_eq!(this.tabs.len(), 1);
    });
    assert!(tree_focused(cx, window, &workspace));

    // Open lib.rs from the tree, then close it.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    });
    assert!(!cx.update(|cx| workspace.read(cx).home));
    assert_eq!(text(cx, &workspace), ORIGINAL);
    step(cx, window, |window, cx| window.press("secondary-w", cx));
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(this.home, "back to the project's page");
        assert_eq!(this.tabs.len(), 1);
        assert!(this.document().path.is_none());
    });
    assert!(tree_focused(cx, window, &workspace));

    // Cmd+S has nothing to save there.
    step(cx, window, |window, cx| window.press("secondary-s", cx));
    assert!(!cx.has_pending_prompt());
}

#[gpui_kit::test]
fn opened_folders_become_recent_projects(cx: &mut TestAppContext) {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    let (window, workspace) = open_empty(cx);
    let ws = workspace.clone();
    let (a, b) = (first.path().to_path_buf(), second.path().to_path_buf());
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.open_folder(&a, window, cx);
            this.open_folder(&b, window, cx);
        })
    });
    assert!(
        cx.update(|cx| workspace.read(cx).home),
        "on the project's page: no file is open yet"
    );
    assert_eq!(
        cx.update(|cx| crate::recent::get(cx)),
        [
            second.path().canonicalize().unwrap(),
            first.path().canonicalize().unwrap()
        ]
    );
}

#[gpui_kit::test]
fn opening_another_project_asks_where(cx: &mut TestAppContext) {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    std::fs::write(first.path().join("lib.rs"), ORIGINAL).unwrap();
    let (window, workspace) = open(cx, &first.path().join("lib.rs"));
    let root = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).project_root(cx));
    let open_second = |cx: &mut TestAppContext| {
        let (ws, dir) = (workspace.clone(), second.path().to_path_buf());
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| this.open_project(&dir, window, cx))
        });
    };

    open_second(cx);
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert_eq!(root(cx), Some(first.path().canonicalize().unwrap()));

    open_second(cx);
    cx.simulate_prompt_answer("New Window");
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| cx.windows().len()), 2);
    assert_eq!(root(cx), Some(first.path().canonicalize().unwrap()));

    open_second(cx);
    cx.simulate_prompt_answer("This Window");
    cx.run_until_parked();
    assert_eq!(root(cx), Some(second.path().canonicalize().unwrap()));
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(this.home, "on the new project's page");
        assert_eq!(this.tabs.len(), 1);
        assert!(this.document().path.is_none(), "the old file is closed");
    });
}

#[gpui_kit::test]
fn settings_float_bottom_left_on_the_home_page(cx: &mut TestAppContext) {
    let (window, _) = open_empty(cx);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let button = vcx.debug_bounds("settings").expect("settings are shown");
    let viewport = vcx.update(|window, _| window.viewport_size());
    assert!(button.left() < px(20.));
    assert!(viewport.height - button.bottom() < px(20.));
}

#[gpui_kit::test]
fn settings_sit_at_the_bottom_of_the_activity_bar(cx: &mut TestAppContext) {
    let (_dir, window, _) = three_files(cx);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let button = vcx.debug_bounds("settings").expect("settings are shown");
    let files = vcx.debug_bounds("activity-files").unwrap();
    assert_eq!(button.left(), files.left());
    let viewport = vcx.update(|window, _| window.viewport_size());
    assert!(viewport.height - button.bottom() < px(20.));
}
