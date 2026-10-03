//! The project around the open file: its root and its `JIG.md` rules.

use std::path::{Path, PathBuf};

pub const RULES_FILE: &str = "JIG.md";

/// Rules beyond this many bytes are cut, so a huge file can't swamp every
/// request.
const MAX_RULES_BYTES: usize = 16 * 1024;

/// A starter for a new `JIG.md`.
pub const RULES_TEMPLATE: &str = "\
# Project rules for Jig

Everything in this file is sent with every Jig command run on files in this
project. Keep it short: conventions, libraries, things the model gets wrong.

- 
";

/// The project root for `file`: the nearest folder above it containing
/// `.git`, or the file's own folder when there is none.
pub fn root_for(file: &Path) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new("."));
    dir.ancestors()
        .find(|dir| dir.join(".git").exists())
        .unwrap_or(dir)
        .to_path_buf()
}

/// The nearest `JIG.md` from `file`'s folder up to the project root.
pub fn rules_path(file: &Path) -> Option<PathBuf> {
    let root = root_for(file);
    let dir = file.parent()?;
    for dir in dir.ancestors() {
        let candidate = dir.join(RULES_FILE);
        if candidate.is_file() {
            return Some(candidate);
        }
        if dir == root {
            break;
        }
    }
    None
}

/// The project rules that apply to `file`, read fresh so edits take effect
/// on the next command.
pub fn rules_for(file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(rules_path(file)?).ok()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.len() <= MAX_RULES_BYTES {
        return Some(text.to_string());
    }
    let mut end = MAX_RULES_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(format!(
        "{}\n[JIG.md was cut here; it is longer than Jig sends.]",
        &text[..end]
    ))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn nearest_rules_up_to_the_git_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let file = root.join("crates/app/src/main.rs");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(&file, "").unwrap();
        // Above the root: never used.
        fs::write(dir.path().join(RULES_FILE), "outside").unwrap();

        assert_eq!(root_for(&file), root);
        assert_eq!(rules_for(&file), None);

        fs::write(root.join(RULES_FILE), "root rules\n").unwrap();
        assert_eq!(rules_for(&file).as_deref(), Some("root rules"));

        fs::write(root.join("crates/app").join(RULES_FILE), "app rules").unwrap();
        assert_eq!(
            rules_for(&file).as_deref(),
            Some("app rules"),
            "the nearest one wins"
        );
    }

    #[test]
    fn without_git_only_the_files_folder_counts() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a/b.rs");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(dir.path().join(RULES_FILE), "parent").unwrap();
        assert_eq!(root_for(&file), dir.path().join("a"));
        assert_eq!(rules_for(&file), None);
        fs::write(dir.path().join("a").join(RULES_FILE), "here").unwrap();
        assert_eq!(rules_for(&file).as_deref(), Some("here"));
    }

    #[test]
    fn empty_rules_are_ignored_and_huge_ones_cut() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.rs");
        fs::write(dir.path().join(RULES_FILE), "  \n").unwrap();
        assert_eq!(rules_for(&file), None);

        fs::write(dir.path().join(RULES_FILE), "é".repeat(MAX_RULES_BYTES)).unwrap();
        let rules = rules_for(&file).unwrap();
        assert!(rules.ends_with("longer than Jig sends.]"));
        assert!(rules.len() < MAX_RULES_BYTES + 100);
    }
}
