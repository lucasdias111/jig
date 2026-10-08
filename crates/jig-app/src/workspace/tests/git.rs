//! Change bars, the Git view and branches.

use super::*;

/// A repository with `a.rs` committed as three lines, open in a window.
fn committed_repo(
    cx: &mut TestAppContext,
) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
    };
    git(&["init", "--quiet", "--initial-branch=main"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "Test"]);
    // Windows runners turn line endings into CRLF on checkout.
    git(&["config", "core.autocrlf", "false"]);
    std::fs::write(dir.path().join("a.rs"), "one\ntwo\nthree\n").unwrap();
    git(&["add", "a.rs"]);
    git(&["commit", "--quiet", "-m", "First"]);
    let (window, workspace) = open(cx, &dir.path().join("a.rs"));
    (dir, window, workspace)
}

#[gpui_kit::test]
fn changed_lines_are_marked_and_a_change_can_be_reverted(cx: &mut TestAppContext) {
    use gpui_kit::base::input::LineChangeKind;
    let (_dir, window, workspace) = committed_repo(cx);
    let rows =
        |cx: &mut TestAppContext| workspace.read_with(cx, |this, cx| this.line_change_rows(cx));
    assert_eq!(rows(cx), vec![]);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            let editor = this.editor().clone();
            editor.apply_edit(4..7, "TWO", window, cx);
            let end = editor.text(cx).len();
            editor.apply_edit(end..end, "four\n", window, cx);
        });
    });
    assert_eq!(
        rows(cx),
        vec![
            (1..2, LineChangeKind::Modified),
            (3..4, LineChangeKind::Added)
        ]
    );

    // Click the bar beside the changed line.
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.update(|window, cx| window.render_frame(cx));
    let second_line = vcx.update(|_, cx| {
        let state = workspace.read(cx).editor().state().read(cx);
        state.range_to_bounds(&(4..4)).unwrap()
    });
    // The bar sits just left of the text.
    let bar = gpui_kit::point(
        second_line.left() - gpui_kit::px(3.),
        second_line.center().y,
    );
    vcx.simulate_click(bar, gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert!(vcx.update(|_, cx| workspace.read(cx).hunk_popup_open()));
    vcx.update(|window, cx| window.render_frame(cx));
    let revert = vcx.debug_bounds("git-revert").expect("the popup shows");
    vcx.simulate_click(revert.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    assert_eq!(text_of(&mut vcx, &workspace), "one\ntwo\nthree\nfour\n");
    assert!(!vcx.update(|_, cx| workspace.read(cx).hunk_popup_open()));
    assert_eq!(
        vcx.update(|_, cx| workspace.read(cx).line_change_rows(cx)),
        vec![(3..4, LineChangeKind::Added)]
    );
}

fn text_of(vcx: &mut gpui_kit::VisualTestContext, workspace: &Entity<Workspace>) -> String {
    vcx.update(|_, cx| workspace.read(cx).editor().text(cx))
}

#[gpui_kit::test]
fn the_git_panel_commits_the_checked_files_and_the_marks_follow(cx: &mut TestAppContext) {
    let (dir, window, workspace) = committed_repo(cx);
    std::fs::write(dir.path().join("b.rs"), "new\n").unwrap();
    std::fs::write(dir.path().join("c.rs"), "also new\n").unwrap();
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            let editor = this.editor().clone();
            editor.apply_edit(0..3, "ONE", window, cx);
            this.save(&crate::workspace::Save, window, cx);
        });
    });
    assert_eq!(
        workspace.read_with(cx, |this, cx| this.line_change_rows(cx).len()),
        1
    );
    step(cx, window, |window, cx| {
        window.dispatch_action(Box::new(crate::workspace::ToggleGitPanel), cx)
    });
    let panel = workspace
        .read_with(cx, |this, _| this.git_panel())
        .expect("the panel opens");
    let files = |cx: &mut TestAppContext| {
        panel.read_with(cx, |panel, _| {
            panel
                .status()
                .map(|status| status.files.iter().map(|f| f.path.clone()).collect())
                .unwrap_or_else(Vec::new)
        })
    };
    assert_eq!(files(cx), vec!["a.rs", "b.rs", "c.rs"]);
    // Changes to tracked files start checked, new files don't.
    let checked = |cx: &mut TestAppContext| panel.read_with(cx, |panel, _| panel.checked_files());
    assert_eq!(checked(cx), vec!["a.rs"]);
    let p = panel.clone();
    step(cx, window, move |_, cx| {
        p.update(cx, |panel, cx| panel.toggle_file("b.rs", cx))
    });
    assert_eq!(checked(cx), vec!["a.rs", "b.rs"]);
    let p = panel.clone();
    step(cx, window, move |window, cx| {
        p.update(cx, |panel, cx| {
            panel.set_message("Shout", window, cx);
            panel.commit(window, cx);
        })
    });
    assert!(!panel.read_with(cx, |panel, _| panel.is_busy()));
    assert_eq!(files(cx), vec!["c.rs"], "the unchecked file stays out");
    assert_eq!(checked(cx), Vec::<String>::new());
    let log = std::process::Command::new("git")
        .arg("-C")
        .arg(dir.path())
        .args(["log", "--format=%s"])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&log.stdout), "Shout\nFirst\n");
    // The commit is the new base: nothing differs from it.
    assert_eq!(
        workspace.read_with(cx, |this, cx| this.line_change_rows(cx)),
        vec![]
    );
}

#[gpui_kit::test]
fn the_activity_bar_switches_and_collapses_the_sidebar(cx: &mut TestAppContext) {
    use crate::workspace::sidebar::SidebarView;
    let (_dir, window, workspace) = committed_repo(cx);
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    let ws = workspace.clone();
    let click = move |vcx: &mut gpui_kit::VisualTestContext, selector: &'static str| {
        // Skip to the end of the sidebar's motion, so nothing moves
        // between finding the button and clicking it.
        vcx.update(|window, cx| {
            ws.update(cx, |this, _| this.sidebar_motion = None);
            window.render_frame(cx)
        });
        let bounds = vcx.debug_bounds(selector).expect(selector);
        vcx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
        vcx.run_until_parked();
    };
    let shown = |vcx: &mut gpui_kit::VisualTestContext| {
        vcx.update(|_, cx| {
            let this = workspace.read(cx);
            this.sidebar_shown().then_some(this.sidebar_view)
        })
    };
    assert_eq!(shown(&mut vcx), None, "a single file opens without it");
    click(&mut vcx, "activity-files");
    assert_eq!(shown(&mut vcx), Some(SidebarView::Files));
    click(&mut vcx, "activity-git");
    assert_eq!(shown(&mut vcx), Some(SidebarView::Git));
    assert!(vcx.update(|_, cx| workspace.read(cx).git_panel().is_some()));
    click(&mut vcx, "activity-git");
    assert_eq!(
        shown(&mut vcx),
        None,
        "the shown view's button collapses it"
    );

    click(&mut vcx, "branch-switcher");
    assert!(vcx.update(|_, cx| workspace.read(cx).branch_picker().is_some()));
}

#[gpui_kit::test]
fn the_git_view_follows_changes_made_elsewhere(cx: &mut TestAppContext) {
    use crate::workspace::git::watch::Touched;
    let (dir, window, workspace) = committed_repo(cx);
    step(cx, window, |window, cx| {
        window.dispatch_action(Box::new(crate::workspace::ToggleGitPanel), cx)
    });
    let panel = workspace
        .read_with(cx, |this, _| this.git_panel())
        .expect("the view opens");
    let files = |cx: &mut TestAppContext| {
        panel.read_with(cx, |panel, _| {
            panel
                .status()
                .map(|status| status.files.iter().map(|f| f.path.clone()).collect())
                .unwrap_or_else(Vec::<String>::new)
        })
    };
    assert_eq!(files(cx), Vec::<String>::new());

    // Another program writes a file; the watcher reports it.
    std::fs::write(dir.path().join("b.rs"), "new\n").unwrap();
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.repo_changed(Touched::Files, window, cx))
    });
    assert_eq!(files(cx), vec!["b.rs"]);

    // A commit in a terminal: the list empties and the bars go.
    std::fs::write(dir.path().join("a.rs"), "changed\n").unwrap();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
    };
    git(&["add", "a.rs", "b.rs"]);
    git(&["commit", "--quiet", "-m", "Elsewhere"]);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| {
            this.reload_changed_files(None, window, cx);
            this.repo_changed(Touched::Git, window, cx)
        })
    });
    assert_eq!(files(cx), Vec::<String>::new());
    assert_eq!(
        workspace.read_with(cx, |this, cx| this.line_change_rows(cx)),
        vec![]
    );
}

#[gpui_kit::test]
fn switching_branches_reloads_open_files(cx: &mut TestAppContext) {
    let (dir, window, workspace) = committed_repo(cx);
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
    };
    git(&["switch", "--quiet", "--create", "other"]);
    std::fs::write(dir.path().join("a.rs"), "other\n").unwrap();
    git(&["commit", "--quiet", "-am", "Other"]);
    git(&["switch", "--quiet", "main"]);

    step(cx, window, |window, cx| {
        window.dispatch_action(Box::new(crate::workspace::SwitchBranch), cx)
    });
    let picker = workspace
        .read_with(cx, |this, _| this.branch_picker())
        .expect("the picker opens");
    // New Branch… first, then the branches, most recent first, but both
    // commits are in the same second.
    let mut names = picker.read_with(cx, |picker, _| picker.row_names());
    assert_eq!(names.remove(0), "New Branch…");
    names.sort();
    assert_eq!(names, vec!["main".to_string(), "other".to_string()]);
    cx.simulate_input(window, "oth");
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |this, _| this.branch_picker().is_none()));
    assert_eq!(text(cx, &workspace), "other\n");
    assert_eq!(
        workspace.read_with(cx, |this, cx| this.line_change_rows(cx)),
        vec![]
    );
}

#[gpui_kit::test]
fn new_branch_from_the_picker_and_the_git_menu(cx: &mut TestAppContext) {
    let (dir, window, workspace) = committed_repo(cx);
    let head = || {
        let output = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["branch", "--show-current"])
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_string()
    };
    let rows = |cx: &mut TestAppContext| {
        let picker = workspace
            .read_with(cx, |this, _| this.branch_picker())
            .unwrap();
        picker.read_with(cx, |picker, _| picker.row_names())
    };

    // The switcher's first row turns it into the name field.
    step(cx, window, |window, cx| {
        window.dispatch_action(Box::new(crate::workspace::SwitchBranch), cx)
    });
    cx.run_until_parked();
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    assert_eq!(rows(cx), Vec::<String>::new());
    cx.simulate_input(window, "main");
    assert_eq!(rows(cx), ["Taken main"], "a taken name isn't offered");
    cx.simulate_keystrokes(window, "escape");
    cx.run_until_parked();

    // Git > New Branch… opens straight to it.
    step(cx, window, |window, cx| {
        window.dispatch_action(Box::new(crate::workspace::NewBranch), cx)
    });
    cx.run_until_parked();
    cx.simulate_input(window, "my feature");
    assert_eq!(rows(cx), ["Create my-feature"]);
    cx.simulate_keystrokes(window, "enter");
    cx.run_until_parked();
    assert!(workspace.read_with(cx, |this, _| this.branch_picker().is_none()));
    assert_eq!(head(), "my-feature");
}

#[gpui_kit::test]
fn the_git_views_branch_button_switches_or_creates_branches(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = committed_repo(cx);
    step(cx, window, |window, cx| {
        window.dispatch_action(Box::new(crate::workspace::ToggleGitPanel), cx)
    });
    // The sidebar slides open on the clock, clipping the panel until
    // it has.
    std::thread::sleep(std::time::Duration::from_millis(250));
    let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
    vcx.run_until_parked();
    vcx.update(|window, cx| window.render_frame(cx));
    let button = vcx
        .debug_bounds("git-branches")
        .expect("the button is in the Git view");
    vcx.simulate_click(button.center(), gpui_kit::Modifiers::none());
    vcx.run_until_parked();
    let picker = vcx
        .update(|_, cx| workspace.read(cx).branch_picker())
        .expect("the switcher opens");
    let rows =
        |vcx: &mut gpui_kit::VisualTestContext| vcx.update(|_, cx| picker.read(cx).row_names());
    assert_eq!(rows(&mut vcx), ["New Branch…", "main"]);
    vcx.simulate_input("second");
    assert_eq!(rows(&mut vcx), ["Create second"]);
}
