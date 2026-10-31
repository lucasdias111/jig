//! The project's files as the pickers list them: everything `.gitignore`
//! doesn't exclude, as paths relative to the project with `/` separators.

use std::path::Path;

use ignore::WalkBuilder;

/// Walk no further than this many files, so a home folder opened as a
/// project can't stall the panel.
const MAX_FILES: usize = 200_000;

/// `path` relative to `root` with `/` separators, if it is inside it.
pub fn relative_path(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let parts: Vec<_> = rest
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Every file under `root` that `.gitignore` doesn't exclude, sorted by
/// path, relative to `root`.
pub fn walk(root: &Path) -> Vec<String> {
    let mut walk = WalkBuilder::new(root);
    walk.hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .require_git(false)
        .parents(true)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(|entry| {
            let name = entry.file_name();
            name != ".git" && name != ".DS_Store"
        });
    walk.build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|entry| relative_path(root, entry.path()))
        .take(MAX_FILES)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    #[test]
    fn walk_skips_ignored_files_and_git() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "").unwrap();
        fs::create_dir_all(root.join("src/ui")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
        fs::write(root.join("src/ui/button.rs"), "").unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        fs::write(root.join("target/debug/jig"), "").unwrap();
        fs::write(root.join("build.log"), "").unwrap();
        fs::write(root.join(".env"), "").unwrap();
        fs::write(root.join(".DS_Store"), "").unwrap();

        assert_eq!(
            walk(root),
            [".env", ".gitignore", "src/lib.rs", "src/ui/button.rs"]
        );
    }
}
