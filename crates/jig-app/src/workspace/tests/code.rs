//! Go to definition, completion, snippets and Rename Symbol.

use super::*;

mod completions;
mod servers;
use completions::completion_labels;

#[gpui_kit::test]
fn f12_goes_to_a_definition_in_another_file_then_to_usages(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() {\n    alpha();\n}\n",
    )
    .unwrap();
    let (window, workspace) = open(cx, dir.path());
    open_file(cx, window, &workspace, &dir.path().join("src/main.rs"));

    // On the call: to the function in lib.rs.
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        editor.select(18..18, cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("f12", cx);
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(
            this.document()
                .path
                .as_ref()
                .unwrap()
                .ends_with("src/lib.rs")
        );
        assert_eq!(this.editor().selection(cx), 7..12);
    });

    // On the declaration: its usages.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("f12", cx);
    });
    let query = cx.update(|cx| {
        let this = workspace.read(cx);
        this.find_in_files
            .read(cx)
            .panel()
            .map(|view| view.read(cx).query(cx))
    });
    assert_eq!(query.as_deref(), Some("alpha"));
}

/// The current tab's text, and what's selected in it.
fn text_and_selection(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> (String, String) {
    cx.update(|cx| {
        let editor = workspace.read(cx).editor().clone();
        let text = editor.text(cx);
        let selected = text[editor.selection(cx)].to_string();
        (text, selected)
    })
}
