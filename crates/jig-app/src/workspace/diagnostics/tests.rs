use gpui_kit::base::input::{Diagnostic, DiagnosticEntry, DiagnosticSeverity};

use super::{on_lines, step};

fn entry(range: std::ops::Range<usize>, message: &str) -> DiagnosticEntry {
    DiagnosticEntry {
        range,
        diagnostic: Diagnostic::new(
            gpui_kit::component::input::Position::new(0, 0)
                ..gpui_kit::component::input::Position::new(0, 0),
            message.to_string(),
        ),
    }
}

#[test]
fn steps_round_the_file() {
    let entries = [
        entry(4..6, "a"),
        entry(10..12, "b"),
        entry(10..20, "b too"),
        entry(30..31, "c"),
    ];
    assert_eq!(step(&entries, 0, true), Some(0));
    assert_eq!(step(&entries, 4, true), Some(1));
    // Problems starting at the same place are one stop.
    assert_eq!(step(&entries, 10, true), Some(3));
    assert_eq!(step(&entries, 30, true), Some(0), "round the end");
    assert_eq!(step(&entries, 30, false), Some(1));
    assert_eq!(step(&entries, 4, false), Some(3), "round the start");
    assert_eq!(step(&[], 0, true), None);
}

#[test]
fn problems_on_the_target_lines() {
    let text = "fn a() {\n    let x: u32 = \"s\";\n}\nfn b() {}\n";
    let line2 = text.find("let").unwrap();
    let line4 = text.find("fn b").unwrap();
    let mut mismatch = entry(line2 + 13..line2 + 16, "mismatched types");
    mismatch.diagnostic.severity = DiagnosticSeverity::Error;
    mismatch.diagnostic.source = Some("rustc".into());
    mismatch.diagnostic.code = Some("E0308".into());
    let mut unused = entry(line4 + 3..line4 + 4, "unused");
    unused.diagnostic.severity = DiagnosticSeverity::Warning;
    let entries = [mismatch, unused];

    // The cursor's line, as a selection-scoped jig targets it.
    let line = line2 - 4..line4 - 3;
    let found = on_lines(&entries, text, line);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].line, 2);
    assert_eq!(found[0].severity, "error");
    assert_eq!(found[0].message, "mismatched types (rustc E0308)");

    // A cursor anywhere on the line counts; whole lines selected with
    // their newline don't reach the next.
    assert_eq!(on_lines(&entries, text, line2..line2).len(), 1);
    assert_eq!(on_lines(&entries, text, 0..line2 - 4).len(), 0);
    assert_eq!(on_lines(&entries, text, 0..text.len()).len(), 2);
    assert_eq!(on_lines(&entries, text, line4..line4)[0].line, 4);
}
