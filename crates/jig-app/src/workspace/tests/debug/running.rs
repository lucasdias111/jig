//! Running a configuration and opening files from its output.

use super::*;

#[gpui_kit::test]
fn runs_a_configuration_and_opens_files_from_its_output(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::create_dir_all(dir.path().join(".jig")).unwrap();
    std::fs::write(dir.path().join("src/lib.rs"), ORIGINAL).unwrap();
    let hello = if cfg!(windows) {
        "echo hello %GREETING%& 1>&2 echo src/lib.rs:1:4: here& exit 2"
    } else {
        "echo hello $GREETING; echo 'src/lib.rs:1:4: here' >&2; exit 2"
    };
    std::fs::write(
        dir.path().join(".jig/run.toml"),
        format!(
            r#"
[[run]]
name = "Other"
command = "exit 0"

[[run]]
name = "Hello"
command = "{hello}"
env = {{ GREETING = "there" }}
"#
        ),
    )
    .unwrap();
    let (window, workspace) = open(cx, dir.path());

    step(cx, window, |window, cx| {
        window.render_frame(cx);
        window.press("ctrl-alt-r", cx);
    });
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).run_picker_rows(cx))
            .is_some_and(|rows| rows.len() == 3)
    });
    assert_eq!(
        cx.update(|cx| workspace.read(cx).run_picker_rows(cx))
            .unwrap(),
        ["Other", "Hello", "Edit run.toml…"]
    );
    step(cx, window, |window, cx| window.input("hel", cx));
    step(cx, window, |window, cx| window.press("enter", cx));
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).run_output(cx))
            .is_some_and(|(_, _, ending)| ending.is_some())
    });
    let (first, lines, ending) = cx.update(|cx| workspace.read(cx).run_output(cx)).unwrap();
    assert_eq!(ending.as_deref(), Some("Process finished with exit code 2"));
    let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
    assert!(texts[0].starts_with("$ echo hello"));
    assert!(texts.contains(&"hello there".to_string()), "{texts:?}");
    assert!(
        texts
            .last()
            .unwrap()
            .starts_with("Process finished with exit code 2 (")
    );

    let link = lines
        .iter()
        .find_map(|line| line.links.first().cloned())
        .expect("the file reference is a link");
    let target = workspace.clone();
    step(cx, window, move |window, cx| {
        target.update(cx, |this, cx| this.open_link(&link, window, cx));
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
        assert_eq!(this.editor().selection(cx), 3..3);
    });

    // ⌃R runs the chosen configuration again.
    step(cx, window, |window, cx| window.press("ctrl-r", cx));
    wait_until(cx, |cx| {
        cx.update(|cx| workspace.read(cx).run_output(cx))
            .is_some_and(|(id, _, ending)| id != first && ending.is_some())
    });
}
