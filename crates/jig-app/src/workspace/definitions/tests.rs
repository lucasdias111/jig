use std::path::{Path, PathBuf};

use super::reference_results;
use jig_lsp::lsp::Encoding;

#[test]
fn references_become_rows() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("b.rs"), "fn b() {\n    alpha();\n}\n").unwrap();
    let at = |line, start, end| {
        lsp_types::Range::new(
            lsp_types::Position::new(line, start),
            lsp_types::Position::new(line, end),
        )
    };
    let found: Vec<(PathBuf, lsp_types::Range)> = vec![
        (root.join("b.rs"), at(1, 4, 9)),
        (root.join("a.rs"), at(0, 7, 12)),
        (root.join("b.rs"), at(1, 4, 9)),
    ];
    // a.rs is open with unsaved text; b.rs is read from disk.
    let buffers = [(root.join("a.rs"), "pub fn alpha() {}\n".to_string())].into();
    let results = reference_results(Path::new(root), found, buffers, Encoding::Utf16);
    let rows: Vec<String> = results
        .matches
        .iter()
        .map(|m| format!("{}:{}: {} {:?}", m.path, m.line + 1, m.text, m.range))
        .collect();
    assert_eq!(
        rows,
        ["a.rs:1: pub fn alpha() {} 7..12", "b.rs:2: alpha(); 13..18"]
    );
    assert_eq!(results.files, 2);
}
