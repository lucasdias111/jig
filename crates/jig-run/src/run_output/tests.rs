use std::fs;

use super::*;

fn red() -> Style {
    Style {
        fg: Some(Color::Indexed(1)),
        ..Default::default()
    }
}

#[test]
fn colours_become_spans_and_escapes_go() {
    let mut style = Style::default();
    let (text, spans) = strip_ansi("\x1b[1m\x1b[31merror\x1b[0m: oops\x1b[K", &mut style);
    assert_eq!(text, "error: oops");
    assert_eq!(
        spans,
        [(
            0..5,
            Style {
                bold: true,
                ..red()
            }
        )]
    );
    assert_eq!(style, Style::default());
}

#[test]
fn colour_carries_over_to_the_next_line() {
    let mut style = Style::default();
    strip_ansi("\x1b[31mstart", &mut style);
    let (_, spans) = strip_ansi("more", &mut style);
    assert_eq!(spans, [(0..4, red())]);
}

#[test]
fn extended_colours_and_hyperlinks() {
    let mut style = Style::default();
    let (text, spans) = strip_ansi(
        "\x1b]8;;file:///x\x1b\\link\x1b]8;;\x1b\\ \x1b[38;2;10;20;30mrgb\x1b[38;5;9m!",
        &mut style,
    );
    assert_eq!(text, "link rgb!");
    assert_eq!(spans[0].1.fg, Some(Color::Rgb(10, 20, 30)));
    assert_eq!(
        spans[1],
        (
            8..9,
            Style {
                fg: Some(Color::Indexed(9)),
                ..Default::default()
            }
        )
    );
}

#[test]
fn carriage_returns_keep_the_last_redraw() {
    let line = parse_line(
        "10%\r50%\r100%\r\n",
        Stream::Stdout,
        &mut Style::default(),
        &[],
    );
    assert_eq!(line.text.as_ref(), "100%");
}

#[test]
fn existing_files_are_linked_with_line_and_column() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/main.rs"), "").unwrap();
    fs::write(root.join("app.py"), "").unwrap();
    let bases = [root.join("sub"), root.clone()];

    let links = find_links("  --> src/main.rs:12:5", &bases);
    assert_eq!(
        links,
        [Link {
            range: 6..22,
            path: root.join("src/main.rs"),
            line: 12,
            column: Some(5),
        }]
    );
    let links = find_links(r#"  File "app.py", line 3, in <module>"#, &bases);
    assert_eq!(links[0].path, root.join("app.py"));
    assert_eq!(links[0].line, 3);

    assert!(find_links("src/missing.rs:1:1 at 12:30:01", &bases).is_empty());
}

#[test]
fn runs_stops_and_reports_the_ending() {
    let dir = tempfile::tempdir().unwrap();
    let config = RunConfig {
        name: "t".into(),
        command: if cfg!(windows) {
            "echo out& 1>&2 echo err& exit 3"
        } else {
            "echo out; echo err >&2; exit 3"
        }
        .into(),
        cwd: dir.path().to_path_buf(),
        env: Vec::new(),
        source: crate::run_configs::Source::File,
        debug: None,
        debugger: None,
    };
    let (tx, rx) = futures::channel::mpsc::unbounded();
    let process = Process::spawn(&config, dir.path(), false, tx).unwrap();
    let events: Vec<RunEvent> = futures::executor::block_on_stream(rx).collect();
    let mut lines: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            RunEvent::Line(line) => Some((line.text.to_string(), line.stream)),
            _ => None,
        })
        .collect();
    lines.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        lines,
        [
            ("err".into(), Stream::Stderr),
            ("out".into(), Stream::Stdout)
        ]
    );
    assert!(matches!(
        events.last(),
        Some(RunEvent::Exited { message, success: false })
            if message == "Process finished with exit code 3"
    ));
    assert!(!process.is_running());

    let config = RunConfig {
        command: if cfg!(windows) {
            "ping -n 30 127.0.0.1 >nul"
        } else {
            "sleep 30"
        }
        .into(),
        ..config
    };
    let (tx, rx) = futures::channel::mpsc::unbounded();
    let process = Process::spawn(&config, dir.path(), false, tx).unwrap();
    process.stop();
    let ending = futures::executor::block_on_stream(rx).find_map(|e| match e {
        RunEvent::Exited { message, .. } => Some(message),
        _ => None,
    });
    assert_eq!(ending.as_deref(), Some("Process stopped"));
}
