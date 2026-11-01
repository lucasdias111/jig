//! The file tree in the sidebar: new files and folders, renaming, revealing and moving several.

use super::*;

fn start_new(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    kind: jig_files::file_tree::NewEntry,
) {
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        let tree = ws.read(cx).tree.as_ref().unwrap().view.clone();
        tree.update(cx, |tree, cx| tree.start_new(kind, window, cx));
    });
}

#[gpui_kit::test]
fn new_file_and_folder_from_the_sidebar(cx: &mut TestAppContext) {
    use jig_files::file_tree::NewEntry;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), ORIGINAL).unwrap();
    let (window, workspace) = open(cx, dir.path());

    // "src" is selected, so the file goes inside it.
    start_new(cx, window, &workspace, NewEntry::File);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("my", cx);
        window.press("space", cx);
        window.input("file.rs", cx);
        window.press("enter", cx);
    });
    let created = dir.path().join("src/my file.rs");
    assert!(created.is_file(), "space types into the name");
    assert_eq!(
        tree_rows(cx, &workspace),
        ["src", "  lib.rs", "  my file.rs"]
    );
    cx.update(|cx| {
        let open = workspace.read(cx).document().path.clone().unwrap();
        assert_eq!(
            open.canonicalize().unwrap(),
            created.canonicalize().unwrap()
        );
    });

    // The new file is selected, so the folder goes next to it.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-shift-e", cx);
    });
    start_new(cx, window, &workspace, NewEntry::Folder);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("ui", cx);
        window.press("enter", cx);
    });
    assert!(dir.path().join("src/ui").is_dir());
    assert!(tree_focused(cx, window, &workspace));
    assert_eq!(tree_rows(cx, &workspace)[1], "  ui", "folders sort first");

    // "ui" is now selected, so this goes inside it. A name that's
    // taken keeps the field open; Escape drops it.
    std::fs::write(dir.path().join("src/ui/taken.rs"), "kept").unwrap();
    start_new(cx, window, &workspace, NewEntry::File);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("taken.rs", cx);
        window.press("enter", cx);
    });
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src/ui/taken.rs")).unwrap(),
        "kept"
    );
    let naming = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            let tree = workspace.read(cx).tree.as_ref().unwrap().view.read(cx);
            tree.naming_error()
        })
    };
    assert_eq!(naming(cx), Some(Some("taken.rs already exists.".into())));
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    });
    assert_eq!(naming(cx), None, "Escape drops the name field");
    assert!(
        tree_focused(cx, window, &workspace),
        "Escape returns to the tree"
    );
    assert_eq!(
        tree_rows(cx, &workspace),
        ["src", "  ui", "    taken.rs", "  lib.rs", "  my file.rs"]
    );
}

#[gpui_kit::test]
fn renaming_in_the_sidebar_moves_the_open_tab(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    let path = dir.path().join("src/lib.rs");
    std::fs::write(&path, ORIGINAL).unwrap();
    let (window, workspace) = open(cx, &path);

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-shift-e", cx);
    });
    assert!(tree_focused(cx, window, &workspace));
    // F2 selects the name without its extension, so typing keeps it.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("f2", cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("main", cx);
        window.press("enter", cx);
    });
    let renamed = dir.path().join("src/main.rs");
    assert!(renamed.is_file() && !path.exists());
    assert_eq!(tree_rows(cx, &workspace), ["src", "  main.rs"]);
    cx.update(|cx| {
        let ws = workspace.read(cx);
        let open = ws.document().path.clone().unwrap();
        assert_eq!(
            open.canonicalize().unwrap(),
            renamed.canonicalize().unwrap()
        );
        assert_eq!(ws.editor().text(cx), ORIGINAL);
    });
}

#[gpui_kit::test]
fn sidebar_toggles_and_reveals_the_open_file(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
    let path = dir.path().join("src/lib.rs");
    std::fs::write(&path, ORIGINAL).unwrap();
    let (window, workspace) = open(cx, &path);

    assert!(
        !cx.update(|cx| workspace.read(cx).sidebar.open),
        "a single file opens without the sidebar"
    );
    assert_eq!(
        tree_rows(cx, &workspace),
        ["src", "  lib.rs", "Cargo.toml"],
        "the project is the git root, with the file revealed"
    );

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-u", cx);
    });
    assert!(cx.update(|cx| workspace.read(cx).sidebar.open));
    assert!(
        !tree_focused(cx, window, &workspace),
        "Cmd+U keeps focus in the editor"
    );

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-shift-e", cx);
    });
    assert!(tree_focused(cx, window, &workspace));
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    });
    assert!(
        !tree_focused(cx, window, &workspace),
        "Escape returns to the editor"
    );

    // A file added on disk shows up when the tree refreshes.
    std::fs::write(dir.path().join("src/new.rs"), "").unwrap();
    cx.update(|cx| workspace.update(cx, |this, cx| this.refresh_tree(cx)));
    assert!(tree_rows(cx, &workspace).contains(&"  new.rs".to_string()));

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-u", cx);
    });
    assert!(!cx.update(|cx| workspace.read(cx).sidebar.open));
}

fn tree_selection(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
    cx.update(|cx| {
        let tree = workspace.read(cx).tree.as_ref().unwrap().view.read(cx);
        tree.selected_names()
    })
}

#[gpui_kit::test]
fn selecting_several_files_and_moving_them_moves_their_tabs(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::create_dir_all(dir.path().join("lib")).unwrap();
    std::fs::write(dir.path().join("a.rs"), ORIGINAL).unwrap();
    std::fs::write(dir.path().join("b.rs"), "").unwrap();
    std::fs::write(dir.path().join("c.rs"), "").unwrap();
    let (window, workspace) = open(cx, &dir.path().join("a.rs"));

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-shift-e", cx);
    });
    assert_eq!(tree_selection(cx, &workspace), ["a.rs"]);
    step(cx, window, |window, cx| {
        window.press("shift-down", cx);
        window.press("shift-down", cx);
    });
    assert_eq!(tree_selection(cx, &workspace), ["a.rs", "b.rs", "c.rs"]);
    step(cx, window, |window, cx| window.press("shift-up", cx));
    assert_eq!(tree_selection(cx, &workspace), ["a.rs", "b.rs"]);
    step(cx, window, |window, cx| window.press("secondary-a", cx));
    assert_eq!(
        tree_selection(cx, &workspace),
        ["lib", "a.rs", "b.rs", "c.rs"]
    );
    step(cx, window, |window, cx| window.press("escape", cx));
    assert_eq!(
        tree_selection(cx, &workspace),
        ["b.rs"],
        "Escape goes back to one row"
    );
    assert!(
        tree_focused(cx, window, &workspace),
        "and stays in the tree"
    );

    step(cx, window, |window, cx| {
        window.press("shift-up", cx);
    });
    let (ws, lib, a) = (
        workspace.clone(),
        dir.path().join("lib").canonicalize().unwrap(),
        dir.path().join("a.rs").canonicalize().unwrap(),
    );
    let into = lib.clone();
    step(cx, window, move |window, cx| {
        let tree = ws.read(cx).tree.as_ref().unwrap().view.clone();
        tree.update(cx, |tree, cx| tree.move_into(&a, &into, window, cx));
    });
    assert_eq!(
        tree_rows(cx, &workspace),
        ["lib", "  a.rs", "  b.rs", "c.rs"]
    );
    assert_eq!(tree_selection(cx, &workspace), ["a.rs", "b.rs"]);
    cx.update(|cx| {
        let ws = workspace.read(cx);
        let open = ws.document().path.clone().unwrap();
        assert_eq!(open.canonicalize().unwrap(), lib.join("a.rs"));
        assert_eq!(ws.editor().text(cx), ORIGINAL);
    });
}
