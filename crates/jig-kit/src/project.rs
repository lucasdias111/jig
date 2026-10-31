//! The project around the open file: its root and its `AGENTS.md`, the
//! instructions file coding agents read.

use std::path::{Path, PathBuf};

pub const AGENTS_FILE: &str = "AGENTS.md";

/// A starter for a new `AGENTS.md`.
pub const AGENTS_TEMPLATE: &str = "\
# AGENTS.md

Instructions for AI tools working in this project. The coding agent Jig
runs reads it. Keep it short: conventions, libraries, how to build and
test, things models get wrong.

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

/// The project files the agent reads besides the code, by name, for the
/// UI.
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
        assert_eq!(agents_path(&file), None);

        fs::write(root.join(AGENTS_FILE), "root rules\n").unwrap();
        assert_eq!(agents_path(&file), Some(root.join(AGENTS_FILE)));

        let app = root.join("crates/app").join(AGENTS_FILE);
        fs::write(&app, "app rules").unwrap();
        assert_eq!(agents_path(&file), Some(app), "the nearest one wins");
        assert_eq!(context_names(&file), [AGENTS_FILE]);
    }

    #[test]
    fn without_git_only_the_files_folder_counts() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a/b.rs");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(dir.path().join(AGENTS_FILE), "parent").unwrap();
        assert_eq!(root_for(&file), dir.path().join("a"));
        assert_eq!(agents_path(&file), None);
        assert!(context_names(&file).is_empty());
        let here = dir.path().join("a").join(AGENTS_FILE);
        fs::write(&here, "here").unwrap();
        assert_eq!(agents_path(&file), Some(here));
    }
}
