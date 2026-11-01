//! Saving, reloading, the file tree, Go to File and Find in Files.

use super::*;

mod finding;
mod sidebar;

#[gpui_kit::test]
fn edit_marks_dirty_and_save_writes_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn a() {}\n").unwrap();
    let (window, workspace) = open(cx, &path);

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        assert_eq!(workspace.read(cx).editor().language(cx), "rust");
        assert!(!workspace.read(cx).tab().dirty);
        window.press(END_OF_FILE, cx);
        window.input("// x\n", cx);
    });
    step(cx, window, |window, cx| {
        assert!(
            workspace.read(cx).tab().dirty,
            "typing marks the buffer dirty"
        );
        window.press("secondary-s", cx);
    });
    step(cx, window, |_, cx| {
        assert!(
            !workspace.read(cx).tab().dirty,
            "saving clears the dirty flag"
        )
    });
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn a() {}\n// x\n");
}

#[gpui_kit::test]
fn files_changed_on_disk_reload_keeping_the_cursor(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn a() {}\nfn b() {}\n").unwrap();
    let (window, workspace) = open(cx, &path);
    let b = "fn a() {}\n".len();
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(b..b, cx);
    });

    std::fs::write(&path, "fn first() {}\nfn a() {}\nfn b() {}\n").unwrap();
    reload_changed_files(cx, window, &workspace);
    assert_eq!(
        text(cx, &workspace),
        "fn first() {}\nfn a() {}\nfn b() {}\n"
    );
    let b = b + "fn first() {}\n".len();
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert_eq!(this.editor().selection(cx), b..b);
        assert!(!this.tab().dirty);
    });
}

#[gpui_kit::test]
fn files_changed_on_disk_keep_unsaved_changes(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn a() {}\n").unwrap();
    let (window, workspace) = open(cx, &path);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("x", cx);
    });

    std::fs::write(&path, "fn other() {}\n").unwrap();
    reload_changed_files(cx, window, &workspace);
    assert_eq!(text(cx, &workspace), "xfn a() {}\n");
    assert!(cx.update(|cx| workspace.read(cx).tab().dirty));
}

#[gpui_kit::test]
fn undoing_back_to_saved_text_clears_dirty(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn a() {}\n").unwrap();
    let (window, workspace) = open(cx, &path);

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("x", cx);
    });
    step(cx, window, |window, cx| {
        assert!(workspace.read(cx).tab().dirty);
        window.press("secondary-z", cx);
    });
    step(cx, window, |_, cx| assert!(!workspace.read(cx).tab().dirty));
}

fn tree_rows(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
    cx.update(|cx| {
        let tree = workspace.read(cx).tree.as_ref().unwrap().view.read(cx);
        tree.model()
            .rows()
            .iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth), row.name))
            .collect()
    })
}

#[gpui_kit::test]
fn opening_a_folder_browses_and_opens_files(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src/ui")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), ORIGINAL).unwrap();
    std::fs::write(dir.path().join("README.md"), "# hi\n").unwrap();
    let (window, workspace) = open(cx, dir.path());

    assert!(cx.update(|cx| workspace.read(cx).sidebar.open));
    assert!(tree_focused(cx, window, &workspace), "starts in the tree");
    assert_eq!(tree_rows(cx, &workspace), ["src", "README.md"]);

    // Expand src, go to lib.rs, open it.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("enter", cx);
    });
    assert_eq!(
        tree_rows(cx, &workspace),
        ["src", "  ui", "  lib.rs", "README.md"]
    );
    step(cx, window, |window, cx| {
        window.press("down", cx);
        window.press("right", cx);
        window.press("down", cx);
        window.press("enter", cx);
    });
    assert_eq!(text(cx, &workspace), ORIGINAL);
    step(cx, window, |window, cx| {
        let this = workspace.read(cx);
        assert!(
            this.document()
                .path
                .as_ref()
                .unwrap()
                .ends_with("src/lib.rs")
        );
        assert!(this.editor().state().focus_handle(cx).is_focused(window));
    });
    assert_eq!(
        tree_rows(cx, &workspace),
        ["src", "  ui", "  lib.rs", "README.md"],
        "an empty folder expands to nothing"
    );
}
