//! The tree's listings: which folders are open and what they contain, read
//! from disk the first time a folder is expanded and again on refresh.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// One file or folder in a listing.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    /// Matched by a `.gitignore`: shown, but dimmed.
    ignored: bool,
}

/// A visible line of the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    pub ignored: bool,
}

/// The tree without any UI: which folders are open and what they contain.
/// Folders are read the first time they're expanded and again on refresh.
pub struct TreeModel {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    listings: HashMap<PathBuf, Vec<Entry>>,
    rows: Vec<Row>,
}

impl TreeModel {
    pub fn new(root: &Path) -> Self {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let mut this = Self {
            root,
            expanded: HashSet::new(),
            listings: HashMap::new(),
            rows: Vec::new(),
        };
        this.rebuild();
        this
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|row| row.path == path)
    }

    pub fn toggle(&mut self, dir: &Path) {
        if !self.expanded.remove(dir) {
            self.expanded.insert(dir.to_path_buf());
        }
        self.rebuild();
    }

    pub fn set_expanded(&mut self, dir: &Path, expanded: bool) {
        let changed = if expanded {
            self.expanded.insert(dir.to_path_buf())
        } else {
            self.expanded.remove(dir)
        };
        if changed {
            self.rebuild();
        }
    }

    /// Expand every folder between the root and `path`, so it has a row.
    /// Returns that row, or `None` when `path` is outside the project.
    pub fn reveal(&mut self, path: &Path) -> Option<usize> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if !path.starts_with(&self.root) || path == self.root {
            return None;
        }
        let mut changed = false;
        for dir in path.ancestors().skip(1) {
            if dir == self.root {
                break;
            }
            changed |= self.expanded.insert(dir.to_path_buf());
        }
        if changed {
            self.rebuild();
        }
        self.index_of(&path)
    }

    /// Read every open folder again, to pick up changes made on disk.
    pub fn refresh(&mut self) {
        self.listings.clear();
        self.expanded.retain(|dir| dir.is_dir());
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        let root = self.root.clone();
        self.push_children(&root, 0, false, &mut rows);
        self.rows = rows;
    }

    fn push_children(&mut self, dir: &Path, depth: usize, ignored: bool, rows: &mut Vec<Row>) {
        let entries = self
            .listings
            .entry(dir.to_path_buf())
            .or_insert_with(|| read_listing(&self.root, dir, ignored))
            .clone();
        for entry in entries {
            let expanded = entry.is_dir && self.expanded.contains(&entry.path);
            rows.push(Row {
                path: entry.path.clone(),
                name: entry.name,
                depth,
                is_dir: entry.is_dir,
                expanded,
                ignored: entry.ignored,
            });
            if expanded {
                self.push_children(&entry.path, depth + 1, entry.ignored, rows);
            }
        }
    }
}

/// The folders and files directly inside `dir`, folders first, each group
/// sorted by name ignoring case. Unreadable folders list as empty.
fn read_listing(root: &Path, dir: &Path, parent_ignored: bool) -> Vec<Entry> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let gitignore = if parent_ignored {
        Gitignore::empty()
    } else {
        gitignore_for(root, dir)
    };
    let mut entries: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter_map(|item| {
            let name = item.file_name().to_string_lossy().into_owned();
            if name == ".git" || name == ".DS_Store" {
                return None;
            }
            let path = item.path();
            // Follows symlinks, so a link to a folder expands like one.
            let is_dir = path.is_dir();
            let ignored = parent_ignored || gitignore.matched(&path, is_dir).is_ignore();
            Some(Entry {
                path,
                name,
                is_dir,
                ignored,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    entries
}

/// The `.gitignore` rules that apply inside `dir`, from the root down.
fn gitignore_for(root: &Path, dir: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(root);
    let mut dirs: Vec<&Path> = dir
        .ancestors()
        .take_while(|d| d.starts_with(root))
        .collect();
    dirs.reverse();
    for dir in dirs {
        let file = dir.join(".gitignore");
        if file.is_file() {
            builder.add(file);
        }
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}
