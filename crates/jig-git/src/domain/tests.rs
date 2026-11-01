use super::hunks::*;
use super::status::*;

#[test]
fn hunks_mark_added_changed_and_deleted_lines() {
    let base = "a\nb\nc\nd\ne\n";
    assert_eq!(hunks(base, base), vec![]);
    assert_eq!(
        hunks(base, "a\nb\nnew\nc\nd\ne\n"),
        vec![Hunk {
            lines: 2..3,
            old_text: String::new()
        }]
    );
    assert_eq!(
        hunks(base, "a\nB\nc\nd\ne\n"),
        vec![Hunk {
            lines: 1..2,
            old_text: "b\n".into()
        }]
    );
    let deleted = hunks(base, "a\nd\ne\n");
    assert_eq!(
        deleted,
        vec![Hunk {
            lines: 1..1,
            old_text: "b\nc\n".into()
        }]
    );
    assert!(deleted[0].is_deletion());
    // At the end, and two runs at once.
    assert_eq!(hunks(base, "a\nb\nc\n").len(), 1);
    assert_eq!(hunks(base, "A\nb\nc\nd\nE\n").len(), 2);
}

#[test]
fn crlf_checkouts_compare_with_lf_buffers() {
    assert_eq!(hunks("a\r\nb\r\n", "a\nb\n"), vec![]);
    assert_eq!(hunks("a\r\nb\r\n", "a\r\nb\r\n"), vec![]);
}

#[test]
fn reads_porcelain_status() {
    let text = [
        "# branch.oid 1234567890abcdef",
        "# branch.head main",
        "# branch.upstream origin/main",
        "# branch.ab +2 -1",
        "1 M. N... 100644 100644 100644 aaaa bbbb src/staged file.rs",
        "1 .M N... 100644 100644 100644 aaaa bbbb README.md",
        "1 MM N... 100644 100644 100644 aaaa bbbb both.rs",
        "2 R. N... 100644 100644 100644 aaaa bbbb R100 new.rs",
        "old.rs",
        "u UU N... 100644 100644 100644 100644 aaaa bbbb cccc conflict.rs",
        "? notes/todo.txt",
        "",
    ]
    .join("\0");
    let status = parse_status(&text);
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.commit.as_deref(), Some("1234567"));
    assert_eq!(status.upstream.as_deref(), Some("origin/main"));
    assert_eq!((status.ahead, status.behind), (2, 1));
    let files: Vec<_> = status
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.staged, f.unstaged))
        .collect();
    assert_eq!(
        files,
        vec![
            ("src/staged file.rs", Some(Change::Modified), None),
            ("README.md", None, Some(Change::Modified)),
            ("both.rs", Some(Change::Modified), Some(Change::Modified)),
            ("new.rs", Some(Change::Renamed), None),
            ("conflict.rs", None, Some(Change::Conflicted)),
            ("notes/todo.txt", None, Some(Change::Untracked)),
        ]
    );
    assert_eq!(status.files[3].original.as_deref(), Some("old.rs"));
    assert!(status.files[5].is_untracked());
}

#[test]
fn a_detached_head_and_a_fresh_repository() {
    let status = parse_status("# branch.oid (initial)\0# branch.head (detached)\0");
    assert_eq!(status.branch, None);
    assert_eq!(status.commit, None);
}

#[test]
fn remote_branches_with_a_local_one_are_left_out() {
    let text = "*\trefs/heads/main\n \trefs/heads/feature\n \trefs/remotes/origin/HEAD\n \trefs/remotes/origin/main\n \trefs/remotes/origin/other\n";
    let names: Vec<_> = parse_branches(text)
        .into_iter()
        .map(|b| (b.name, b.remote, b.current))
        .collect();
    assert_eq!(
        names,
        vec![
            ("main".into(), false, true),
            ("feature".into(), false, false),
            ("origin/other".into(), true, false),
        ]
    );
}

#[test]
fn explains_failures_without_hints() {
    assert_eq!(
        explain(
            "error: failed to push\nhint: try pulling\nfatal: no way\n",
            ""
        ),
        "failed to push\nno way"
    );
    assert_eq!(explain("", "nothing to commit\n"), "nothing to commit");
}
