//! Quick runs: the preview, accepting and rejecting, and what reaches the model.

use super::*;

#[gpui_kit::test]
fn reply_is_previewed_in_place(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = preview_docs(cx);
    step(cx, window, |window, cx| {
        let this = workspace.read(cx);
        assert!(
            this.palette.is_none(),
            "running a command closes the palette"
        );
        assert!(
            this.editor().state().focus_handle(cx).is_focused(window),
            "focus returns to the editor"
        );
        let run = this.run.as_ref().expect("a command ran");
        assert_eq!(run.target, 0..9);
        assert_eq!(
            run.preview.as_ref().unwrap().range,
            0..12,
            "only the added doc line, not the untouched function"
        );
        assert_eq!(
            run.bubble,
            Bubble::Preview {
                message: "Added a doc comment.".into(),
                removed: String::new(),
                agent: false,
            },
            "nothing was removed"
        );
        assert_eq!(
            this.editor().highlighted_ranges(cx),
            vec![0..12],
            "only the new line is highlighted"
        );
        assert_eq!(
            this.editor().text(cx),
            DOCUMENTED,
            "the change is in the buffer"
        );
        assert!(
            !this.editor().state().read(cx).is_editable(),
            "and locked while under review"
        );
    });
}

#[gpui_kit::test]
fn enter_accepts_and_one_undo_reverts(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = preview_docs(cx);
    step(cx, window, |window, cx| window.press("enter", cx));
    assert_eq!(bubble(cx, &workspace), None);
    assert_eq!(
        text(cx, &workspace),
        DOCUMENTED,
        "Enter keeps the change and inserts nothing"
    );
    assert!(cx.update(|cx| workspace.read(cx).editor().state().read(cx).is_editable()));

    step(cx, window, |window, cx| window.press("secondary-z", cx));
    assert_eq!(
        text(cx, &workspace),
        ORIGINAL,
        "one undo reverts the whole command"
    );
}

#[gpui_kit::test]
fn typing_is_blocked_during_review(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = preview_docs(cx);
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("zzz", cx);
    });
    assert_eq!(text(cx, &workspace), DOCUMENTED);
    assert!(bubble(cx, &workspace).is_some_and(|b| b.is_preview()));
}

#[gpui_kit::test]
fn tab_accepts(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = preview_docs(cx);
    step(cx, window, |window, cx| window.press("tab", cx));
    assert_eq!(bubble(cx, &workspace), None);
    assert_eq!(
        text(cx, &workspace),
        DOCUMENTED,
        "Tab keeps the change and inserts nothing"
    );
}

#[gpui_kit::test]
fn escape_rejects(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = preview_docs(cx);
    step(cx, window, |window, cx| window.press("escape", cx));
    assert_eq!(bubble(cx, &workspace), None);
    assert_eq!(text(cx, &workspace), ORIGINAL);
    assert!(
        !cx.update(|cx| workspace.read(cx).tab().dirty),
        "back to the saved text"
    );

    // The editor is usable again.
    step(cx, window, |window, cx| window.input("x", cx));
    assert!(text(cx, &workspace).contains('x'));
}

#[gpui_kit::test]
fn undo_during_review_rejects(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = preview_docs(cx);
    step(cx, window, |window, cx| window.press("secondary-z", cx));
    assert_eq!(bubble(cx, &workspace), None);
    assert_eq!(text(cx, &workspace), ORIGINAL);
}

#[gpui_kit::test]
fn reject_keeps_earlier_edits(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, ORIGINAL).unwrap();
    let (window, workspace) = open(cx, &path);
    use_provider(
        cx,
        &workspace,
        Ok(r#"{"replace": "fn b() {}", "message": "Renamed."}"#),
    );
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press(END_OF_FILE, cx);
        window.input("// kept\n", cx);
    });
    run_preset(cx, window, &workspace, 0..9, "simplify");
    assert_eq!(text(cx, &workspace), "fn b() {}\n// kept\n");
    step(cx, window, |window, cx| window.press("escape", cx));
    assert_eq!(
        text(cx, &workspace),
        "fn a() {}\n// kept\n",
        "only the command is undone"
    );
}

#[gpui_kit::test]
fn unchanged_reply_only_shows_the_message(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, ORIGINAL).unwrap();
    let (window, workspace) = open(cx, &path);
    use_provider(
        cx,
        &workspace,
        Ok(r#"{"replace": "fn a() {}", "message": "Defines an empty function a."}"#),
    );
    run_preset(cx, window, &workspace, 0..9, "explain");
    assert_eq!(
        bubble(cx, &workspace),
        Some(Bubble::Message("Defines an empty function a.".into()))
    );
    assert_eq!(text(cx, &workspace), ORIGINAL);
    assert!(cx.update(|cx| workspace.read(cx).editor().state().read(cx).is_editable()));
}

#[gpui_kit::test]
fn new_command_accepts_pending_change(cx: &mut TestAppContext) {
    let (_dir, window, workspace) = preview_docs(cx);
    step(cx, window, |window, cx| window.press("secondary-k", cx));
    let this_text = text(cx, &workspace);
    assert_eq!(this_text, DOCUMENTED);
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(this.palette.is_some());
        assert!(this.editor().state().read(cx).is_editable());
    });
}

#[gpui_kit::test]
fn waiting_tints_the_target_until_cancelled(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, ORIGINAL).unwrap();
    let (window, workspace) = open(cx, &path);
    use_provider(cx, &workspace, Ok(DOCS_REPLY));

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        workspace.update(cx, |this, cx| {
            this.editor()
                .state()
                .update(cx, |s, cx| s.set_selected_range(0..9, cx))
        });
        window.press("secondary-k", cx);
    });
    // Run without letting the request finish.
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.input("docs", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(this.run.as_ref().unwrap().bubble.is_running());
        assert_eq!(this.editor().highlighted_ranges(cx), vec![0..9]);
    });
    cx.update_window(window, |_, window, cx| window.press("escape", cx))
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert!(this.run.is_none());
        assert!(this.editor().highlighted_ranges(cx).is_empty());
        assert_eq!(
            this.editor().text(cx),
            ORIGINAL,
            "a cancelled request changes nothing"
        );
    });
}

#[gpui_kit::test]
fn provider_error_shows_and_fades(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn a() {}\n").unwrap();
    let (window, workspace) = open(cx, &path);
    use_provider(cx, &workspace, Err("401 Unauthorized: invalid api key"));

    run_preset(cx, window, &workspace, 0..9, "docs");
    assert_eq!(
        bubble(cx, &workspace),
        Some(Bubble::Error("401 Unauthorized: invalid api key".into()))
    );
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(5));
    cx.run_until_parked();
    assert_eq!(bubble(cx, &workspace), None);
}

#[gpui_kit::test]
fn malformed_reply_is_an_error(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn a() {}\n").unwrap();
    let (window, workspace) = open(cx, &path);
    use_provider(cx, &workspace, Ok("Sure, here is the code: fn a() {}"));

    run_preset(cx, window, &workspace, 0..9, "docs");
    assert!(matches!(bubble(cx, &workspace), Some(Bubble::Error(_))));
    assert_eq!(
        cx.update(|cx| workspace.read(cx).editor().text(cx)),
        "fn a() {}\n"
    );
}

#[gpui_kit::test]
fn reply_for_a_changed_buffer_is_discarded(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn a() {}\n").unwrap();
    let (window, workspace) = open(cx, &path);
    use_provider(cx, &workspace, Ok(r#"{"replace": "x", "message": "m"}"#));

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        workspace.update(cx, |this, cx| {
            this.editor()
                .state()
                .update(cx, |s, cx| s.set_selected_range(0..9, cx))
        });
        window.press("secondary-k", cx);
    });
    // Start the command; its request is queued but has not run yet.
    cx.update_window(window, |_, window, cx| {
        window.render_frame(cx);
        window.input("docs", cx);
        window.press("enter", cx);
    })
    .unwrap();
    // Edit while the request is in flight, then let it finish.
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        assert!(ws.read(cx).run.as_ref().unwrap().bubble.is_running());
        ws.update(cx, |this, cx| {
            this.editor().apply_edit(10..10, "// typed\n", window, cx);
        });
    });
    match bubble(cx, &workspace) {
        Some(Bubble::Error(message)) => assert!(message.contains("changed"), "{message}"),
        other => panic!("expected an error, got {other:?}"),
    }
}

#[gpui_kit::test]
fn missing_api_key_is_reported(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn a() {}\n").unwrap();
    let (window, workspace) = open(cx, &path);
    cx.update(|cx| {
        workspace.update(cx, |this, _| {
            this.provider = Err("OPENCODE_API_KEY is not set.".into())
        })
    });

    run_preset(cx, window, &workspace, 0..9, "docs");
    assert_eq!(
        bubble(cx, &workspace),
        Some(Bubble::Error("OPENCODE_API_KEY is not set.".into()))
    );
}

#[gpui_kit::test]
fn escape_closes_palette(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("lib.rs");
    std::fs::write(&path, "fn a() {}\n").unwrap();
    let (window, workspace) = open(cx, &path);

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-k", cx);
    });
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("escape", cx);
    });
    step(cx, window, |window, cx| {
        let this = workspace.read(cx);
        assert!(this.palette.is_none());
        assert!(this.run.is_none());
        assert!(this.editor().state().focus_handle(cx).is_focused(window));
    });
}

#[gpui_kit::test]
fn note_reaches_the_model(cx: &mut TestAppContext) {
    let (_dir, window, workspace, commands) = open_with_commands(cx);
    std::fs::create_dir_all(commands.parent().unwrap()).unwrap();
    std::fs::write(
            &commands,
            "[[jig]]\nname = \"Create model\"\nscope = \"cursor\"\nprompt = \"Insert a model.\"\ncomment = \"required\"\n",
        )
        .unwrap();
    let provider = Arc::new(RecordingProvider(Default::default()));
    let provider_for_ws: Arc<dyn Provider> = provider.clone();
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |this, cx| {
            this.provider = Ok(provider_for_ws);
            this.reload_presets(window, cx);
        })
    })
    .unwrap();

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("secondary-k", cx);
    });
    // Typed and confirmed in one frame, like a fast paste.
    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.input("model", cx);
        window.press("enter", cx);
    });
    step(cx, window, |window, cx| {
        window.input("User with an email", cx);
        window.press("enter", cx);
    });

    let sent = provider.0.lock().unwrap().clone();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].contains("Instruction: Insert a model.\nNote: User with an email\n"),
        "{}",
        sent[0]
    );
}

#[gpui_kit::test]
fn agents_md_goes_with_every_quick_command(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".git")).unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "Use thiserror for errors.\n").unwrap();
    let path = dir.path().join("src/lib.rs");
    std::fs::write(&path, ORIGINAL).unwrap();
    let (window, workspace) = open(cx, &path);
    let provider = Arc::new(RecordingProvider(Default::default()));
    let provider_for_ws: Arc<dyn Provider> = provider.clone();
    cx.update(|cx| workspace.update(cx, |this, _| this.provider = Ok(provider_for_ws)));

    run_preset(cx, window, &workspace, 0..9, "docs");
    let first = provider.0.lock().unwrap()[0].clone();
    assert!(
        first.starts_with("<project_rules>\nUse thiserror for errors.\n</project_rules>"),
        "{first}"
    );

    // Edits to AGENTS.md apply to the next command without a restart.
    step(cx, window, |window, cx| window.press("escape", cx));
    std::fs::write(dir.path().join("AGENTS.md"), "Prefer anyhow.\n").unwrap();
    run_preset(cx, window, &workspace, 0..9, "docs");
    let second = provider.0.lock().unwrap()[1].clone();
    assert!(second.contains("Prefer anyhow."), "{second}");
}
