use super::*;

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

/// A real repository: commit, change, stage, unstage, branch.
#[test]
fn works_with_a_real_repository() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let run = |args: &[&str]| {
        let output = command(&root).args(args).output().unwrap();
        assert!(output.status.success(), "{args:?}: {output:?}");
    };
    run(&["init", "--quiet", "--initial-branch=main"]);
    run(&["config", "user.email", "test@example.com"]);
    run(&["config", "user.name", "Test"]);
    // Windows runners turn line endings into CRLF on checkout.
    run(&["config", "core.autocrlf", "false"]);
    let file = root.join("a.txt");
    std::fs::write(&file, "one\ntwo\n").unwrap();
    let repo = Repo::discover(&file).unwrap();
    assert_eq!(repo.root(), root);
    assert_eq!(repo.committed_text(&file), None);
    assert_eq!(
        repo.status().unwrap().files[0].unstaged,
        Some(Change::Untracked)
    );

    repo.stage(&["a.txt".into()]).unwrap();
    repo.unstage(&["a.txt".into()]).unwrap();
    assert_eq!(
        repo.status().unwrap().files[0].unstaged,
        Some(Change::Untracked)
    );
    let a = vec!["a.txt".to_string()];
    assert!(repo.commit("  ", &a).is_err());
    assert!(repo.commit("First", &[]).is_err());
    repo.commit("First", &a).unwrap();
    assert_eq!(repo.committed_text(&file).as_deref(), Some("one\ntwo\n"));
    let status = repo.status().unwrap();
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert!(status.files.is_empty());

    std::fs::write(&file, "one\n2\n").unwrap();
    repo.stage(&["a.txt".into()]).unwrap();
    assert_eq!(
        repo.status().unwrap().files[0].staged,
        Some(Change::Modified)
    );
    repo.unstage(&["a.txt".into()]).unwrap();
    assert_eq!(repo.status().unwrap().files[0].staged, None);

    // A folder of untracked files is one entry.
    std::fs::create_dir(root.join("build")).unwrap();
    for name in ["x", "y", "z"] {
        std::fs::write(root.join("build").join(name), name).unwrap();
    }
    let paths = |repo: &Repo| -> Vec<String> {
        let status = repo.status().unwrap();
        status.files.into_iter().map(|f| f.path).collect()
    };
    assert_eq!(paths(&repo), vec!["a.txt", "build/"]);
    repo.stage(&["build/".into()]).unwrap();
    assert_eq!(repo.status().unwrap().files.len(), 4);
    repo.unstage(&["build/".into()]).unwrap();
    assert_eq!(paths(&repo), vec!["a.txt", "build/"]);
    // One with thousands is build output, and stays out.
    for n in 0..=MAX_NEW_FOLDER_FILES {
        std::fs::write(root.join("build").join(format!("f{n}")), "").unwrap();
    }
    let error = repo.stage(&["build/".into()]).unwrap_err().to_string();
    assert!(error.starts_with("build/ holds 1004 new files"), "{error}");
    assert!(repo.commit("Build", &["build/".into()]).is_err());
    assert_eq!(paths(&repo), vec!["a.txt", "build/"]);
    std::fs::remove_dir_all(root.join("build")).unwrap();

    // Only the files named are committed; another staged one stays staged.
    std::fs::write(root.join("b.txt"), "b\n").unwrap();
    std::fs::write(root.join("c.txt"), "c\n").unwrap();
    repo.stage(&["c.txt".into()]).unwrap();
    repo.commit("Add b", &["b.txt".into()]).unwrap();
    let status = repo.status().unwrap();
    let left: Vec<_> = status
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.staged))
        .collect();
    assert_eq!(left, vec![("a.txt", None), ("c.txt", Some(Change::Added))]);
    repo.unstage(&["c.txt".into()]).unwrap();
    std::fs::remove_file(root.join("c.txt")).unwrap();

    // Another git holding the index for a moment only delays staging.
    let lock = root.join(".git/index.lock");
    std::fs::write(&lock, "").unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        std::fs::remove_file(lock).unwrap();
    });
    repo.stage(&["a.txt".into()]).unwrap();
    release.join().unwrap();
    repo.unstage(&["a.txt".into()]).unwrap();

    repo.create_branch("feature").unwrap();
    assert_eq!(repo.status().unwrap().branch.as_deref(), Some("feature"));
    let main = Branch {
        name: "main".into(),
        remote: false,
        current: false,
    };
    repo.switch(&main).unwrap();
    let branches = repo.branches().unwrap();
    assert!(branches.iter().any(|b| b.name == "main" && b.current));
    assert!(repo.pull().unwrap_err().to_string().contains("no upstream"));
    assert!(repo.push().unwrap_err().to_string().contains("no remote"));
}
