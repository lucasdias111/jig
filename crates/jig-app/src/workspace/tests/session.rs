//! Each project's open files coming back when its folder opens again.

use super::*;

/// Another window on `path` in the same app, so it shares what's remembered.
fn open_again(cx: &mut TestAppContext, path: &Path) -> Entity<Workspace> {
    let path = path.to_path_buf();
    let (_, workspace) = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            cx.new(|cx| Workspace::new(Some(path), window, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    workspace
}

fn tabs(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> (Vec<String>, usize) {
    cx.update(|cx| {
        let this = workspace.read(cx);
        (this.tab_labels(), this.active)
    })
}

#[gpui_kit::test]
fn reopening_a_folder_brings_back_its_tabs(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.rs", "b.rs", "c.rs"] {
        std::fs::write(dir.path().join(name), format!("// {name}\n")).unwrap();
    }
    let (window, workspace) = open(cx, dir.path());
    for name in ["a.rs", "b.rs", "c.rs"] {
        open_file(cx, window, &workspace, &dir.path().join(name));
    }
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        ws.update(cx, |this, cx| this.activate(1, window, cx))
    });
    std::fs::remove_file(dir.path().join("a.rs")).unwrap();

    let again = open_again(cx, dir.path());
    assert_eq!(tabs(cx, &again), (vec!["b.rs".into(), "c.rs".into()], 0));
    assert!(!cx.update(|cx| again.read(cx).home));
}

#[gpui_kit::test]
fn closing_every_tab_leaves_the_project_page(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "// a\n").unwrap();
    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &dir.path().join("a.rs"));
    press(cx, window, "secondary-w");
    assert!(cx.update(|cx| workspace.read(cx).home));

    let again = open_again(cx, dir.path());
    assert!(cx.update(|cx| again.read(cx).home));
}
