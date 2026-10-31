use super::*;

#[test]
fn applies_a_unified_diff() {
    let old = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
    let diff = "Index: /p/lib.rs\n===================================================================\n--- /p/lib.rs\n+++ /p/lib.rs\n@@ -1,3 +1,4 @@\n+/// Adds two integers.\n fn add(a: i32, b: i32) -> i32 {\n     a + b\n }\n";
    assert_eq!(
        apply_diff(old, diff).unwrap(),
        "/// Adds two integers.\nfn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n"
    );
}

#[test]
fn applies_several_hunks_and_removals() {
    let old = "a\nb\nc\nd\ne\nf\ng\nh\n";
    let diff = "--- x\n+++ x\n@@ -1,2 +1,2 @@\n-a\n+A\n b\n@@ -7,2 +7,1 @@\n g\n-h\n";
    assert_eq!(apply_diff(old, diff).unwrap(), "A\nb\nc\nd\ne\nf\ng\n");
}

#[test]
fn handles_a_missing_final_newline() {
    let old = "one\ntwo";
    let diff = "--- x\n+++ x\n@@ -1,2 +1,2 @@\n one\n-two\n\\ No newline at end of file\n+three\n\\ No newline at end of file\n";
    assert_eq!(apply_diff(old, diff).unwrap(), "one\nthree");
}

#[test]
fn finds_a_hunk_that_moved() {
    let old = "x\ny\na\nb\n";
    let diff = "--- x\n+++ x\n@@ -1,2 +1,2 @@\n a\n-b\n+B\n";
    assert_eq!(apply_diff(old, diff).unwrap(), "x\ny\na\nB\n");
}

#[test]
fn refuses_a_diff_that_doesnt_fit() {
    let diff = "--- x\n+++ x\n@@ -1,1 +1,1 @@\n-nope\n+yes\n";
    assert!(apply_diff("something else\n", diff).is_err());
}

#[test]
fn prompt_says_where_the_user_is() {
    let text = "fn a() {}\nfn b() {}\nfn c() {}\n".to_string();
    let request = Prompt {
        task: None,
        instruction: "Add docs".into(),
        language: "rust".into(),
        file_name: Some("src/lib.rs".into()),
        target: 10..20,
        text,
        diagnostics: Vec::new(),
    };
    assert_eq!(
        prompt(&request),
        format!(
            "Add docs\n\nI'm in src/lib.rs, with line 2 selected:\n```rust\nfn b() {{}}\n```{LINK_THE_CODE}"
        )
    );
    let at_cursor = Prompt {
        target: 20..20,
        ..request
    };
    assert!(prompt(&at_cursor).contains("cursor on line 3. New code goes there."));
    let with_problems = Prompt {
        diagnostics: vec![Diagnostic {
            line: 3,
            severity: "error".into(),
            message: "expected\n `;`".into(),
        }],
        ..at_cursor
    };
    assert!(
        prompt(&with_problems).starts_with(
            "Add docs\nThe language server reports:\n- line 3, error: expected `;`\n\n"
        )
    );
    let step = Prompt {
        task: Some("# Plan\n\n## Todo\n\n- [ ] Add docs ← this step".into()),
        ..with_problems
    };
    let asked = prompt(&step);
    assert!(asked.starts_with("Add docs\n\nThis is from a planned task"));
    assert!(asked.contains("update_task"));
    assert!(asked.contains("<task>\n# Plan"));
}

#[test]
fn replacements_apply_to_the_whole_file_or_a_part() {
    let replace = |old: Option<&str>, new: &str| EditChange::Replace {
        old: old.map(str::to_string),
        new: new.into(),
    };
    let disk = "a\nb\nc\n";
    assert_eq!(replace(Some(disk), "x\n").apply(disk).unwrap(), "x\n");
    assert_eq!(
        replace(Some("b\n"), "B\n").apply(disk).unwrap(),
        "a\nB\nc\n"
    );
    assert_eq!(replace(None, "new\n").apply("").unwrap(), "new\n");
    assert!(replace(Some("zzz"), "y").apply(disk).is_err());
    assert!(replace(Some("\n"), "y").apply(disk).is_err(), "ambiguous");
    assert!(EditChange::Write("w".into()).written_by_jig());
}

#[test]
fn messages_carry_the_code_they_refer_to() {
    let reference = CodeReference {
        file: "src/a.rs".into(),
        language: "rust".into(),
        lines: (10, 12),
        code: "fn a() {\n}\n".into(),
    };
    assert_eq!(reference.name(), "src/a.rs:10-12");
    let sent = message(
        "Why @src/a.rs:10-12?",
        std::slice::from_ref(&reference),
        true,
    );
    assert!(sent.contains("@src/a.rs:10-12:\n```rust\nfn a() {\n}\n```"));
    assert!(sent.ends_with(LINK_THE_CODE));
    assert_eq!(asked(&sent), "Why @src/a.rs:10-12?");
    assert_eq!(message(" Hi ", &[], false), "Hi");
    let one = CodeReference {
        lines: (3, 3),
        ..reference
    };
    assert_eq!(one.name(), "src/a.rs:3");
}

#[test]
fn titles_are_the_start_of_what_was_asked() {
    assert_eq!(title("Add docs\nmore"), "Add docs");
    let long = "word ".repeat(30);
    assert_eq!(title(&long).chars().count(), TITLE_CHARS);
    assert!(title(&long).ends_with('…'));
}
