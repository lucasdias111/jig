//! Keeps Jig's source files short enough to read in one go: no `.rs` file
//! under `crates/` may pass `LIMIT` lines, tests included, since a test
//! module can move to a file of its own (`#[cfg(test)] mod tests;`).
//!
//! Files longer than that today are listed in `file-length-baseline.txt`
//! with their length, which may only come down: a listed file can't grow,
//! and one that has come under the limit leaves the list.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

const LIMIT: usize = 500;
const BLESS: &str = "JIG_LENGTH_BLESS";
const BASELINE: &str = "file-length-baseline.txt";

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn source_files_stay_short() {
    let root = repository();
    let lengths = lengths(&root);
    let over: BTreeMap<&String, &usize> = lengths.iter().filter(|(_, n)| **n > LIMIT).collect();
    let baseline_path = root.join(BASELINE);
    if std::env::var_os(BLESS).is_some() {
        fs::write(&baseline_path, baseline_text(&over)).unwrap();
        return;
    }
    let baseline = read_baseline(&baseline_path);

    let mut problems = Vec::new();
    for (path, &&length) in &over {
        match baseline.get(*path) {
            None => problems.push(format!("{path} has {length} lines, over {LIMIT}; split it")),
            Some(&allowed) if length > allowed => problems.push(format!(
                "{path} grew to {length} lines from {allowed}; split it rather than add to it"
            )),
            Some(&allowed) if length < allowed => problems.push(format!(
                "{path} came down to {length} lines from {allowed}; lower it in {BASELINE}"
            )),
            Some(_) => {}
        }
    }
    for path in baseline.keys() {
        if !over.contains_key(path) {
            problems.push(format!(
                "{path} is under {LIMIT} lines or gone; take it out of {BASELINE}"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "{}\n\nTo accept the lengths as they now are, run {BLESS}=1 cargo test -p jig-app --test file_length",
        problems.join("\n")
    );
}

/// Every `.rs` file under `crates/`, by its path from the repository's root.
fn lengths(root: &Path) -> BTreeMap<String, usize> {
    let mut lengths = BTreeMap::new();
    let mut folders = vec![root.join("crates")];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name != "target") {
                    folders.push(path);
                }
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                let text = fs::read_to_string(&path).unwrap();
                let name = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                lengths.insert(name, text.lines().count());
            }
        }
    }
    lengths
}

fn read_baseline(path: &Path) -> BTreeMap<String, usize> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
        .map(|line| {
            let (path, length) = line
                .rsplit_once(' ')
                .unwrap_or_else(|| panic!("{BASELINE}: {line}"));
            (
                path.to_string(),
                length
                    .parse()
                    .unwrap_or_else(|_| panic!("{BASELINE}: {line}")),
            )
        })
        .collect()
}

fn baseline_text(over: &BTreeMap<&String, &usize>) -> String {
    let mut text = format!(
        "# Source files over {LIMIT} lines, kept until they're split. Each line is `path lines`,\n\
         # and a file's length may only come down.\n\
         # Written by {BLESS}=1 cargo test -p jig-app --test file_length.\n"
    );
    for (path, length) in over {
        writeln!(text, "{path} {length}").unwrap();
    }
    text
}

#[test]
fn reads_the_baseline_it_writes() {
    let (a, b) = ("crates/a.rs".to_string(), "crates/b c.rs".to_string());
    let over = BTreeMap::from([(&a, &600), (&b, &700)]);
    let folder = std::env::temp_dir().join(format!("jig-length-{}", std::process::id()));
    fs::create_dir_all(&folder).unwrap();
    let path = folder.join(BASELINE);
    fs::write(&path, baseline_text(&over)).unwrap();
    let read = read_baseline(&path);
    fs::remove_dir_all(&folder).unwrap();
    assert_eq!(read, BTreeMap::from([(a, 600), (b, 700)]));
}
