//! The project around the open file: its root and its `AGENTS.md`, the
//! instructions file coding agents read. Quick commands get it too, so both
//! lanes follow the same rules.

use std::path::{Path, PathBuf};

pub const AGENTS_FILE: &str = "AGENTS.md";

/// An `AGENTS.md` beyond this many bytes is cut, so a huge file can't swamp
/// every request.
const MAX_AGENTS_BYTES: usize = 16 * 1024;

/// A starter for a new `AGENTS.md`.
pub const AGENTS_TEMPLATE: &str = "\
# AGENTS.md

Instructions for AI tools working in this project. Jig sends this file with
every command; OpenCode and other agents read it too. Keep it short:
conventions, libraries, how to build and test, things models get wrong.

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

/// The nearest `AGENTS.md` from `file`'s folder up to the project root.
pub fn agents_path(file: &Path) -> Option<PathBuf> {
    let root = root_for(file);
    let dir = file.parent()?;
    for dir in dir.ancestors() {
        let candidate = dir.join(AGENTS_FILE);
        if candidate.is_file() {
            return Some(candidate);
        }
        if dir == root {
            break;
        }
    }
    None
}

/// The `AGENTS.md` that applies to `file`, read fresh so edits take effect
/// on the next command. Trimmed, and cut when it's very long.
pub fn agents_for(file: &Path) -> Option<String> {
    let text = std::fs::read_to_string(agents_path(file)?).ok()?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if text.len() <= MAX_AGENTS_BYTES {
        return Some(text.to_string());
    }
    let mut end = MAX_AGENTS_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Some(format!(
        "{}\n[{AGENTS_FILE} was cut here; it is longer than Jig sends.]",
        &text[..end]
    ))
}

/// The project files commands are sent with, by name, for the UI.
pub fn context_names(file: &Path) -> Vec<String> {
    agents_path(file)
        .map(|_| vec![AGENTS_FILE.to_string()])
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn nearest_agents_md_up_to_the_git_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        let file = root.join("crates/app/src/main.rs");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(&file, "").unwrap();
        // Above the root: never used.
        fs::write(dir.path().join(AGENTS_FILE), "outside").unwrap();

        assert_eq!(root_for(&file), root);
        assert_eq!(agents_for(&file), None);

        fs::write(root.join(AGENTS_FILE), "root rules\n").unwrap();
        assert_eq!(agents_for(&file).as_deref(), Some("root rules"));

        fs::write(root.join("crates/app").join(AGENTS_FILE), "app rules").unwrap();
        assert_eq!(
            agents_for(&file).as_deref(),
            Some("app rules"),
            "the nearest one wins"
        );
    }

    #[test]
    fn without_git_only_the_files_folder_counts() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a/b.rs");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(dir.path().join(AGENTS_FILE), "parent").unwrap();
        assert_eq!(root_for(&file), dir.path().join("a"));
        assert_eq!(agents_for(&file), None);
        fs::write(dir.path().join("a").join(AGENTS_FILE), "here").unwrap();
        assert_eq!(agents_for(&file).as_deref(), Some("here"));
    }

    #[test]
    fn empty_agents_md_is_ignored_and_a_huge_one_cut() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.rs");
        fs::write(dir.path().join(AGENTS_FILE), "  \n").unwrap();
        assert_eq!(agents_for(&file), None);

        fs::write(dir.path().join(AGENTS_FILE), "é".repeat(MAX_AGENTS_BYTES)).unwrap();
        let rules = agents_for(&file).unwrap();
        assert!(rules.ends_with("longer than Jig sends.]"));
        assert!(rules.len() < MAX_AGENTS_BYTES + 100);
    }
}
