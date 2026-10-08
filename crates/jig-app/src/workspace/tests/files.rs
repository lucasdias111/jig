//! Saving, reloading, the file tree, Go to File and Find in Files.

use super::*;

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

    assert!(cx.update(|cx| workspace.read(cx).sidebar_open));
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

fn quick_open_rows(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
    cx.update(|cx| {
        let open = workspace.read(cx).quick_open.as_ref().unwrap();
        open.view.read(cx).row_paths()
    })
}

#[gpui_kit::test]
fn go_to_file_finds_and_opens_files(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src/ui")).unwrap();
    std::fs::create_dir_all(dir.path().join("target")).unwrap();
    std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), ORIGINAL).unwrap();
    std::fs::write(dir.path().join("src/ui/button.rs"), "").unwrap();
    std::fs::write(dir.path().join("target/lib.rs"), "").unwrap();
    std::fs::write(dir.path().join("README.md"), "# hi\n").unwrap();
    let (window, workspace) = open(cx, dir.path());

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-p", cx);
    });
    assert_eq!(
        quick_open_rows(cx, &workspace),
        [".gitignore", "README.md", "src/lib.rs", "src/ui/button.rs"],
        "ignored files are left out"
    );

    step(cx, window, |window, cx| window.input("lib", cx));
    assert_eq!(quick_open_rows(cx, &workspace), ["src/lib.rs"]);
    step(cx, window, |window, cx| window.press("enter", cx));
    assert!(cx.update(|cx| workspace.read(cx).quick_open.is_none()));
    assert_eq!(text(cx, &workspace), ORIGINAL);

    // Open a second file; the first is now the most recent other one.
    step(cx, window, |window, cx| {
        window.press("secondary-p", cx);
        window.input("btn", cx);
        window.press("enter", cx);
    });
    step(cx, window, |window, cx| window.press("secondary-p", cx));
    assert_eq!(
        quick_open_rows(cx, &workspace)[0],
        "src/lib.rs",
        "recent files come first, without the current one"
    );
    step(cx, window, |window, cx| window.press("escape", cx));
    step(cx, window, |window, cx| {
        let this = workspace.read(cx);
        assert!(this.quick_open.is_none());
        assert!(this.editor().state().focus_handle(cx).is_focused(window));
        assert!(
            this.document()
                .path
                .as_ref()
                .unwrap()
                .ends_with("src/ui/button.rs")
        );
    });
}

#[gpui_kit::test]
fn find_in_files_searches_the_project_and_opens_the_match(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::create_dir_all(dir.path().join("target")).unwrap();
    std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() {\n    let x = 1;\n    lib::alpha();\n}\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("target/out.rs"), "alpha").unwrap();
    let (window, workspace) = open(cx, dir.path());

    // Into the editor on lib.rs, with `alpha` selected.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        // Typed and chosen before the project's first walk is done.
        window.press("secondary-p", cx);
        window.input("lib", cx);
        window.press("enter", cx);
    });
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(7..12, cx);
    });

    // ⇧⌘F from the editor, which GPUI Kit binds to Replace.
    step(cx, window, |window, cx| {
        window.press("secondary-shift-f", cx)
    });
    assert_eq!(
        cx.update(|cx| workspace
            .read(cx)
            .find_in_files
            .as_ref()
            .unwrap()
            .view
            .read(cx)
            .query(cx)),
        "alpha",
        "the selection is the query"
    );
    assert_eq!(
        find_results(cx, &workspace),
        [
            "src/lib.rs:1: pub fn alpha() {}",
            "src/main.rs:3: lib::alpha();"
        ],
        "ignored files aren't searched"
    );

    step(cx, window, |window, cx| {
        window.press("down", cx);
        window.press("enter", cx);
    });
    step(cx, window, |window, cx| {
        let this = workspace.read(cx);
        assert!(this.find_in_files.is_none());
        assert!(
            this.document()
                .path
                .as_ref()
                .unwrap()
                .ends_with("src/main.rs")
        );
        let text = this.editor().text(cx);
        assert_eq!(&text[this.editor().selection(cx)], "alpha");
        assert!(this.editor().state().focus_handle(cx).is_focused(window));
    });

    // Reopening starts from the last search; options narrow it.
    step(cx, window, |window, cx| {
        window.press("secondary-shift-f", cx)
    });
    step(cx, window, |window, cx| window.input("let", cx));
    assert_eq!(find_results(cx, &workspace), ["src/main.rs:2: let x = 1;"]);
    step(cx, window, |window, cx| {
        window.press("secondary-a", cx);
        window.input("ALPHA", cx);
        window.press("alt-c", cx);
    });
    assert!(find_results(cx, &workspace).is_empty(), "match case is on");
    step(cx, window, |window, cx| window.press("escape", cx));
    assert!(cx.update(|cx| workspace.read(cx).find_in_files.is_none()));
}

#[gpui_kit::test]
fn find_in_files_keeps_the_file_name_in_view(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let long = format!("let needle = \"{}\";\n", "x".repeat(400));
    std::fs::write(dir.path().join("long_file_name.rs"), long).unwrap();
    let (window, workspace) = open(cx, dir.path());
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-shift-f", cx);
    });
    step(cx, window, |window, cx| window.input("needle", cx));
    assert_eq!(find_results(cx, &workspace).len(), 1);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        let row = window.find(("find-in-files-row", 0usize)).bounds();
        let file = window.find(("find-in-files-file", 0usize)).bounds();
        let width = px(crate::find_in_files::WIDTH);
        assert!(row.size.width < width, "the row {row:?} fits the panel");
        assert!(
            file.right() <= row.right() && file.size.width > px(0.),
            "file name {file:?} outside its row {row:?}"
        );
    });
}

fn start_new(
    cx: &mut TestAppContext,
    window: AnyWindowHandle,
    workspace: &Entity<Workspace>,
    kind: crate::file_tree::NewEntry,
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
    use crate::file_tree::NewEntry;
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
        !cx.update(|cx| workspace.read(cx).sidebar_open),
        "a single file opens without the sidebar"
    );
    assert_eq!(
        tree_rows(cx, &workspace),
        ["src", "  lib.rs", "Cargo.toml"],
        "the project is the git root, with the file revealed"
    );

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-b", cx);
    });
    assert!(cx.update(|cx| workspace.read(cx).sidebar_open));
    assert!(
        !tree_focused(cx, window, &workspace),
        "Cmd+B keeps focus in the editor"
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
        window.press("secondary-b", cx);
    });
    assert!(!cx.update(|cx| workspace.read(cx).sidebar_open));
}
