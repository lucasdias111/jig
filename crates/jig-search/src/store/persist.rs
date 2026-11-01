//! The project on disk: its files' text for Find in Files, and the walk
//! listing them.

use std::path::{Path, PathBuf};

use crate::domain::text_search::ProjectText;

/// Files larger than this are skipped: likely generated, and slow to show.
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// The project's folder, read for searching.
pub struct ProjectFolder {
    root: PathBuf,
}

impl ProjectFolder {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }
}

impl ProjectText for ProjectFolder {
    /// The file's text, unless it's huge, binary or not UTF-8.
    fn text(&self, path: &str) -> Option<String> {
        let full = self.root.join(path);
        if std::fs::metadata(&full).map_or(true, |m| m.len() > MAX_FILE_BYTES) {
            return None;
        }
        let bytes = std::fs::read(&full).ok()?;
        if bytes[..bytes.len().min(8192)].contains(&0) {
            return None;
        }
        String::from_utf8(bytes).ok()
    }
}

/// The text of `path` under `root`, to preview a match in.
pub fn read(root: &Path, path: &str) -> Option<String> {
    std::fs::read_to_string(root.join(path)).ok()
}

/// Every file in the project that `.gitignore` leaves, relative to `root`.
pub fn walk(root: &Path) -> Vec<String> {
    jig_commands::project_files::walk(root)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::domain::text_search::{SearchOptions, pattern, search};

    const TEXT: &str = "fn main() {\n    let tab = Tab::new();\n    tabs.push(tab);\n}\n";

    #[test]
    fn search_skips_binary_and_counts_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), TEXT).unwrap();
        fs::write(dir.path().join("b.bin"), b"tab\0tab").unwrap();
        fs::write(dir.path().join("c.md"), "tab here\n").unwrap();
        let files = ["a.rs", "b.bin", "c.md"].map(String::from);
        let regex = pattern("tab", SearchOptions::default()).unwrap();
        let results = search(
            &ProjectFolder::new(dir.path()),
            &files,
            &regex,
            &AtomicBool::new(false),
        );
        assert_eq!(results.files, 2);
        assert_eq!(results.total_lines, 3);
        assert_eq!(results.matches[2].path, "c.md");
    }
}
