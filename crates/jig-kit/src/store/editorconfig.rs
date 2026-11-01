//! Reading the `.editorconfig` files above a file, up to one that says
//! `root = true`.

use std::fs;
use std::path::Path;

use crate::domain::editorconfig::{self, EditorConfig, FILE_NAME};

impl EditorConfig {
    /// What the `.editorconfig` files above `path` say about it. Files that
    /// can't be read count as not there.
    pub fn for_file(path: &Path) -> Self {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut found = Vec::new();
        for dir in path.ancestors().skip(1) {
            let Ok(text) = fs::read_to_string(dir.join(FILE_NAME)) else {
                continue;
            };
            let file = editorconfig::parse(&text);
            let root = file.root;
            found.push((dir.to_path_buf(), file));
            if root {
                break;
            }
        }
        Self::merged(&path, &found)
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::input::TabSize;

    use super::*;
    use crate::domain::editorconfig::LineEnding;

    fn write(dir: &Path, name: &str, text: &str) {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn nearer_files_and_later_sections_win() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".editorconfig",
            "root = true\n\n[*]\nindent_style = space\nindent_size = 4\n\
             trim_trailing_whitespace = true\n\n[*.md]\nindent_size = 2\n\
             trim_trailing_whitespace = false\n",
        );
        write(
            dir.path(),
            "web/.editorconfig",
            "# Not root: the one above still counts.\n[*.md]\nindent_style = TAB\n",
        );
        write(
            dir.path(),
            "vendor/.editorconfig",
            "[*]\nindent_size = unset\nend_of_line = crlf\n",
        );
        let detected = TabSize {
            tab_size: 8,
            hard_tabs: false,
        };

        let rust = EditorConfig::for_file(&dir.path().join("src/main.rs"));
        assert_eq!(rust.tab_size(detected).tab_size, 4);
        assert_eq!(rust.trim_trailing_whitespace(), Some(true));
        assert_eq!(rust.insert_final_newline(), None);

        let readme = EditorConfig::for_file(&dir.path().join("README.md"));
        assert_eq!(readme.tab_size(detected).tab_size, 2);
        assert_eq!(readme.trim_trailing_whitespace(), Some(false));

        let docs = EditorConfig::for_file(&dir.path().join("web/docs.md"));
        let tab = docs.tab_size(detected);
        assert!(tab.hard_tabs, "values are case-insensitive");
        assert_eq!(tab.tab_size, 2, "a tab's width follows indent_size");

        let vendored = EditorConfig::for_file(&dir.path().join("vendor/lib.c"));
        assert_eq!(vendored.tab_size(detected).tab_size, 8, "unset: detected");
        assert_eq!(vendored.end_of_line(), Some(LineEnding::CrLf));
    }

    #[test]
    fn root_stops_the_search() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".editorconfig",
            "root = true\n[*]\nindent_size = 3\n",
        );
        write(dir.path(), "inner/.editorconfig", "root = true\n");
        let config = EditorConfig::for_file(&dir.path().join("inner/a.rs"));
        assert_eq!(config, EditorConfig::default());
    }

    #[test]
    fn indent_size_tab_means_the_tab_width() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".editorconfig",
            "root = true\n[*.go]\nindent_style = tab\nindent_size = tab\ntab_width = 8\n",
        );
        let tab = EditorConfig::for_file(&dir.path().join("main.go")).tab_size(TabSize {
            tab_size: 4,
            hard_tabs: false,
        });
        assert_eq!((tab.tab_size, tab.hard_tabs), (8, true));
    }
}
