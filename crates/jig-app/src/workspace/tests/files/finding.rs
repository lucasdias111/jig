//! Go to File and Find in Files.

use super::*;

fn quick_open_rows(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
    cx.update(|cx| {
        let view = workspace.read(cx).quick_open.read(cx).panel().unwrap();
        view.read(cx).row_paths()
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
    assert!(cx.update(|cx| !workspace.read(cx).quick_open.read(cx).is_open()));
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
        assert!(!this.quick_open.read(cx).is_open());
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
            .read(cx)
            .panel()
            .unwrap()
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
        assert!(!this.find_in_files.read(cx).is_open());
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
    assert!(cx.update(|cx| !workspace.read(cx).find_in_files.read(cx).is_open()));
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
        let width = px(jig_search::find_in_files::WIDTH);
        assert!(row.size.width < width, "the row {row:?} fits the panel");
        assert!(
            file.right() <= row.right() && file.size.width > px(0.),
            "file name {file:?} outside its row {row:?}"
        );
    });
}
