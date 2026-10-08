//! Format Document and what saving does.

use super::*;

/// A Rust file under `dir` served by a fake server answering formatting
/// requests with `answer`, open in a window once the server has said it
/// formats.
fn open_with_formatting_server(
    cx: &mut TestAppContext,
    dir: &Path,
    text: &str,
    answer: impl Fn(&str, &serde_json::Value) -> serde_json::Value + Send + 'static,
) -> (AnyWindowHandle, Entity<Workspace>, std::path::PathBuf) {
    cx.executor().allow_parking();
    // Only the server formats, never what happens to be installed.
    crate::formatters::use_for_tests("");
    // As the workspace has it, so the server is found for the file.
    let dir = &dir.canonicalize().unwrap();
    let path = dir.join("main.rs");
    std::fs::write(&path, text).unwrap();
    let (client, _seen) = crate::lsp::fake_server(dir.to_path_buf(), answer);
    let server = crate::lsp::server_for("rust").unwrap();
    let root = crate::lsp::root_for(server, &path);
    cx.update(|cx| crate::lsp::register(server.name, root, client.clone(), cx));
    for _ in 0..500 {
        if client.formatting().document {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(client.formatting().document, "the server started");
    let (window, workspace) = open(cx, &path);
    (window, workspace, path)
}

#[gpui_kit::test]
fn format_document_makes_the_servers_edits_as_one_undo_step(cx: &mut TestAppContext) {
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let at = |line: u32, start: u32, end: u32| json!({"start": {"line": line, "character": start}, "end": {"line": line, "character": end}});
    let original = "fn a(){\nlet x=1;\n}\n";
    let (window, workspace, _) =
        open_with_formatting_server(cx, dir.path(), original, move |method, params| {
            match method {
                // Last first, as servers often send them.
                "textDocument/formatting" => {
                    assert_eq!(params["options"]["insertSpaces"], true);
                    json!([
                        {"range": at(1, 5, 6), "newText": " = "},
                        {"range": at(1, 0, 0), "newText": "    "},
                        {"range": at(0, 6, 6), "newText": " "},
                    ])
                }
                // Only the selected line, which it was told.
                "textDocument/rangeFormatting" => {
                    assert_eq!(params["range"], at(1, 0, 8));
                    json!([{"range": at(1, 0, 0), "newText": "\t"}])
                }
                _ => serde_json::Value::Null,
            }
        });
    let editor = cx.update(|cx| workspace.read(cx).editor().clone());
    cx.update(|cx| editor.select(8..16, cx));
    press(cx, window, "alt-shift-f");
    wait_until(cx, |cx| {
        cx.update(|cx| editor.text(cx)) == "fn a(){\n\tlet x=1;\n}\n"
    });
    step(cx, window, |window, cx| editor.undo(window, cx));
    cx.update(|cx| editor.select(12..12, cx));
    press(cx, window, "alt-shift-f");
    let formatted = "fn a() {\n    let x = 1;\n}\n";
    wait_until(cx, |cx| cx.update(|cx| editor.text(cx)) == formatted);
    // Still on the `x`.
    let x = formatted.find('x').unwrap();
    assert_eq!(cx.update(|cx| editor.selection(cx)), x..x);
    step(cx, window, |window, cx| editor.undo(window, cx));
    assert_eq!(cx.update(|cx| editor.text(cx)), original, "one undo step");
}

#[gpui_kit::test]
fn format_document_leaves_a_change_under_review_alone(cx: &mut TestAppContext) {
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let (window, workspace, _) = open_with_formatting_server(
        cx,
        dir.path(),
        "a\nb\n",
        |method, _| match method {
            "textDocument/formatting" => {
                json!([{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "x"}])
            }
            _ => serde_json::Value::Null,
        },
    );
    use_provider(
        cx,
        &workspace,
        Ok(r#"{"replace": "B", "message": "Capitalised."}"#),
    );
    run_preset(cx, window, &workspace, 2..3, "Simplify");
    assert!(cx.update(|cx| workspace.read(cx).previewing()));
    press(cx, window, "alt-shift-f");
    std::thread::sleep(std::time::Duration::from_millis(50));
    cx.run_until_parked();
    let text = cx.update(|cx| workspace.read(cx).editor().text(cx));
    assert_eq!(text, "a\nB\n", "not formatted");
}

#[gpui_kit::test]
fn saving_gives_up_on_a_slow_formatter(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let (window, workspace, path) =
        open_with_formatting_server(cx, dir.path(), "fn a() {}\n", |method, _| {
            if method == "textDocument/formatting" {
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
            serde_json::Value::Null
        });
    cx.update(|cx| crate::settings::update(cx, |s| s.formatting.set_on_save("rust", true)));
    let editor = cx.update(|cx| workspace.read(cx).editor().clone());
    step(cx, window, |window, cx| {
        editor.apply_edit(0..0, "// hi\n", window, cx);
    });
    press(cx, window, "secondary-s");
    // Waiting on the formatter, not yet saved.
    assert!(cx.update(|cx| workspace.read(cx).pending_save.is_some()));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn a() {}\n");

    cx.executor()
        .advance_clock(crate::workspace::format::SAVE_TIMEOUT);
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "// hi\nfn a() {}\n"
    );
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(!this.tab().dirty);
        assert!(this.pending_save.is_none());
        let note = this.status_note.as_ref().unwrap();
        assert_eq!(
            note.message.as_ref(),
            "Saved without formatting. The formatter took more than 2 s."
        );
    });
}

#[gpui_kit::test]
fn trims_and_ends_with_a_newline_on_save_when_asked(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes.txt");
    std::fs::write(&path, "a  \nb\t").unwrap();
    let (window, workspace) = open(cx, &path);
    press(cx, window, "secondary-s");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "a  \nb\t", "off");
    cx.update(|cx| {
        crate::settings::update(cx, |s| {
            s.formatting.trim_trailing_whitespace = true;
            s.formatting.final_newline = true;
        })
    });
    press(cx, window, "secondary-s");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\nb\n");
    let text = cx.update(|cx| workspace.read(cx).editor().text(cx));
    assert_eq!(text, "a\nb\n", "the buffer too");
}

/// With no language server, the formatter for the language, fed the
/// buffer on stdin: on save, and with Format Document.
#[cfg(unix)]
#[gpui_kit::test]
fn without_a_server_the_formatter_formats(cx: &mut TestAppContext) {
    cx.executor().allow_parking();
    crate::formatters::use_for_tests(
        r#"
[[formatter]]
key = "missing"
name = "Missing"
languages = ["rust"]
command = ["jig-no-such-formatter"]

[[formatter]]
key = "upper"
name = "Upper"
languages = ["rust"]
command = ["/bin/sh", "-c", "tr a-z A-Z"]
"#,
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("main.rs");
    std::fs::write(&path, "let x = 1;  \nlet y = 2;").unwrap();
    let (window, workspace) = open(cx, &path);
    cx.update(|cx| {
        crate::settings::update(cx, |s| {
            s.formatting.set_on_save("rust", true);
            s.formatting.trim_trailing_whitespace = true;
            s.formatting.final_newline = true;
        })
    });
    press(cx, window, "secondary-s");
    wait_until(cx, |_| {
        std::fs::read_to_string(&path).unwrap() == "LET X = 1;\nLET Y = 2;\n"
    });

    let editor = cx.update(|cx| workspace.read(cx).editor().clone());
    step(cx, window, |window, cx| {
        editor.apply_edit(0..0, "fn b() {}\n", window, cx);
        editor.select(3..3, cx);
    });
    press(cx, window, "alt-shift-f");
    wait_until(cx, |cx| cx.update(|cx| editor.text(cx)).starts_with("FN B"));
    assert_eq!(
        cx.update(|cx| editor.selection(cx)),
        3..3,
        "the cursor stays"
    );
}
