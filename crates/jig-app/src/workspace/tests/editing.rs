//! Line shortcuts, Go to Line, comments, and Find and Replace.

use super::*;

#[gpui_kit::test]
fn toggle_line_comment_comments_and_uncomments(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.py");
    std::fs::write(&path, "def a():\n    return 1\n").unwrap();
    let (window, workspace) = open(cx, &path);
    let editor = cx.update(|cx| workspace.read(cx).editor().clone());
    step(cx, window, |window, cx| {
        editor.focus(window, cx);
        editor.select(0..0, cx);
    });
    step(cx, window, |window, cx| window.press("secondary-'", cx));
    assert_eq!(text(cx, &workspace), "# def a():\n    return 1\n");
    step(cx, window, |window, cx| {
        window.dispatch_action(Box::new(crate::workspace::ToggleLineComment), cx)
    });
    assert_eq!(text(cx, &workspace), "def a():\n    return 1\n");
}

#[gpui_kit::test]
fn line_shortcuts_work_in_the_code_editor(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "one\ntwo\nthree").unwrap();
    let (window, workspace) = open(cx, &path);
    let editor = cx.update(|cx| workspace.read(cx).editor().clone());
    step(cx, window, |window, cx| {
        editor.focus(window, cx);
        editor.select(0..0, cx);
    });
    step(cx, window, |window, cx| window.press("alt-down", cx));
    assert_eq!(text(cx, &workspace), "two\none\nthree");
    step(cx, window, |window, cx| {
        window.press("secondary-shift-d", cx)
    });
    assert_eq!(text(cx, &workspace), "two\none\none\nthree");
    step(cx, window, |window, cx| {
        window.press("secondary-shift-backspace", cx)
    });
    assert_eq!(text(cx, &workspace), "two\none\nthree");
    // The cursor is on the line that took the deleted one's place.
    step(cx, window, |window, cx| window.press("alt-up", cx));
    assert_eq!(text(cx, &workspace), "two\nthree\none");
    step(cx, window, |window, cx| window.press("secondary-l", cx));
    assert_eq!(cx.update(|cx| editor.selection(cx)), 4..9);
}

#[gpui_kit::test]
fn go_to_line_moves_the_cursor(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("a.txt");
    std::fs::write(&path, "one\ntwo\nthree").unwrap();
    let (window, workspace) = open(cx, &path);
    step(cx, window, |window, cx| {
        window.dispatch_action(Box::new(crate::workspace::GoToLine), cx)
    });
    assert!(cx.update(|cx| workspace.read(cx).go_to_line.is_some()));
    step(cx, window, |window, cx| window.input("3:2", cx));
    step(cx, window, |window, cx| window.press("enter", cx));
    assert!(cx.update(|cx| workspace.read(cx).go_to_line.is_none()));
    let editor = cx.update(|cx| workspace.read(cx).editor().clone());
    assert_eq!(cx.update(|cx| editor.selection(cx)), 9..9);
}

/// The find panel's query, or `None` while it's closed.
fn find_query(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Option<String> {
    cx.update(|cx| {
        workspace
            .read(cx)
            .find_panel
            .as_ref()
            .map(|open| open.view.read(cx).query(cx))
    })
}

#[gpui_kit::test]
fn find_starts_from_the_selection_and_enter_goes_to_the_next_match(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "let foo = 1;\nfoo += foo;\n").unwrap();
    let (window, workspace) = open(cx, &path);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        workspace.read(cx).editor().clone().select(4..7, cx);
        window.press("secondary-f", cx);
    });
    assert_eq!(find_query(cx, &workspace).as_deref(), Some("foo"));
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        let this = workspace.read(cx);
        assert!(
            this.find_panel_focused(window, cx),
            "typing goes to the panel"
        );
        let session = this.editor().state().read(cx).search_session();
        assert!(session.is_active(), "the matches are highlighted");
        assert_eq!(session.matcher.len(), 3);
        assert_eq!(
            session.matcher.current(),
            Some(0),
            "starting at the selection"
        );
    });
    assert_eq!(selection(cx, &workspace), 4..7);

    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(
        selection(cx, &workspace),
        13..16,
        "the next match is selected"
    );
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(selection(cx, &workspace), 20..23);
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(selection(cx, &workspace), 4..7, "round to the first");
    step(cx, window, |window, cx| window.press("shift-enter", cx));
    assert_eq!(selection(cx, &workspace), 20..23, "and back");

    step(cx, window, |window, cx| window.press("escape", cx));
    assert_eq!(find_query(cx, &workspace), None, "Esc closes it");
    step(cx, window, |window, cx| {
        let this = workspace.read(cx);
        assert!(this.editor().state().focus_handle(cx).is_focused(window));
        assert!(!this.editor().state().read(cx).search_session().is_active());
    });
    assert_eq!(selection(cx, &workspace), 20..23, "at the last match");

    // A selection over two lines isn't a query: the last one is.
    step(cx, window, |window, cx| {
        workspace.read(cx).editor().clone().select(0..16, cx);
        window.press("secondary-f", cx);
    });
    assert_eq!(find_query(cx, &workspace).as_deref(), Some("foo"));
}

#[gpui_kit::test]
fn find_and_replace_replaces_every_match_as_one_step(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "foo bar foo\nfoo\n").unwrap();
    let (window, workspace) = open(cx, &path);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        workspace.read(cx).editor().clone().select(0..3, cx);
        window.press("secondary-alt-f", cx);
    });
    assert_eq!(find_query(cx, &workspace).as_deref(), Some("foo"));
    assert!(cx.update(|cx| {
        workspace
            .read(cx)
            .find_panel
            .as_ref()
            .unwrap()
            .view
            .read(cx)
            .is_replacing()
    }));
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        // The replace field has the keyboard.
        window.input("baz", cx);
    });
    step(cx, window, |window, cx| {
        window.press("secondary-alt-enter", cx)
    });
    assert_eq!(text(cx, &workspace), "baz bar baz\nbaz\n");
    assert!(cx.update(|cx| workspace.read(cx).tab().dirty));

    step(cx, window, |window, cx| window.press("escape", cx));
    step(cx, window, |window, cx| window.press("secondary-z", cx));
    assert_eq!(text(cx, &workspace), "foo bar foo\nfoo\n", "one undo step");
}

#[gpui_kit::test]
fn replace_is_refused_while_a_change_is_under_review(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = preview_docs(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-alt-f", cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        // From the replace field to the query.
        window.press("tab", cx);
        window.input("a()", cx);
    });
    step(cx, window, |window, cx| {
        window.press("tab", cx);
        window.input("b()", cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-alt-enter", cx);
        window.press("enter", cx);
    });
    assert_eq!(text(cx, &workspace), DOCUMENTED, "nothing replaced");
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert_eq!(
            this.editor()
                .state()
                .read(cx)
                .search_session()
                .matcher
                .len(),
            1,
            "though there was a match"
        );
        assert!(this.previewing(), "and Enter didn't accept the change");
    });

    // Esc closes the panel first, then rejects the change.
    step(cx, window, |window, cx| window.press("escape", cx));
    assert_eq!(find_query(cx, &workspace), None);
    assert!(cx.update(|cx| workspace.read(cx).previewing()));
    step(cx, window, |window, cx| window.press("escape", cx));
    assert_eq!(text(cx, &workspace), ORIGINAL);
}

#[gpui_kit::test]
fn the_palette_closes_the_find_panel(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, ORIGINAL).unwrap();
    let (window, workspace) = open(cx, &path);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-f", cx);
    });
    assert!(find_query(cx, &workspace).is_some());
    step(cx, window, |window, cx| window.press("secondary-k", cx));
    assert_eq!(find_query(cx, &workspace), None);
    assert!(cx.update(|cx| workspace.read(cx).palette.is_some()));
    // ⌘F leaves the palette alone.
    step(cx, window, |window, cx| window.press("secondary-f", cx));
    assert_eq!(find_query(cx, &workspace), None);
}
