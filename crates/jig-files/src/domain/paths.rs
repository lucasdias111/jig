//! The rules for making, renaming and moving files in the tree, on paths
//! alone: which names are allowed, where moved paths end up, and what a
//! rename field starts with selected.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};

/// What the New File and New Folder buttons make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewEntry {
    File,
    Folder,
}

/// `name`, trimmed, as a path inside a folder: it may contain `/` to make
/// folders on the way, like `src/ui/tree.rs`, but never leave the folder.
pub fn new_name(name: &str) -> Result<&Path, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a name.".into());
    }
    let relative = Path::new(name);
    if !relative
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err("Use a name inside this folder.".into());
    }
    Ok(relative)
}

/// `name`, trimmed, as a new name for an entry in the same folder: one
/// component, no slashes.
pub fn rename_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a name.".into());
    }
    let mut components = Path::new(name).components();
    if !matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(_)), None)
    ) {
        return Err("Use a name without slashes.".into());
    }
    Ok(name)
}

/// Whether renaming `path` to `name` only changes its case, which is
/// allowed over the name it has, as Finder allows it.
pub fn same_but_case(path: &Path, name: &str) -> bool {
    path.file_name()
        .is_some_and(|old| old.to_string_lossy().to_lowercase() == name.to_lowercase())
}

/// Where `path` is now that `from` has moved to `to`, if it was inside it.
pub fn moved_path(path: &Path, from: &Path, to: &Path) -> Option<PathBuf> {
    path.strip_prefix(from).ok().map(|rest| {
        if rest.as_os_str().is_empty() {
            to.to_path_buf()
        } else {
            to.join(rest)
        }
    })
}

/// `paths` without those inside another of them, which go along with it.
pub fn outermost(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|path| {
            !paths
                .iter()
                .any(|other| other != *path && path.starts_with(other))
        })
        .cloned()
        .collect()
}

/// What moving entries into a folder did: where each one went, and why it
/// stopped short, if it did.
#[derive(Debug, Default, PartialEq)]
pub struct Moved {
    pub moves: Vec<(PathBuf, PathBuf)>,
    pub error: Option<String>,
}

/// The last part of `path`'s name, as shown.
pub fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Where each of `paths` goes when moved into `dir`, keeping its name, or
/// why none can: a name already taken (`taken` says whether a path is) or
/// a folder dropped into one inside it. Entries already in `dir`, or `dir`
/// itself, stay put, and entries inside another one go along with it.
pub fn plan_move(
    paths: &[PathBuf],
    dir: &Path,
    taken: impl Fn(&Path) -> bool,
) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let paths: Vec<PathBuf> = outermost(paths)
        .into_iter()
        .filter(|path| path.parent() != Some(dir) && path != dir)
        .collect();
    let mut targets = HashSet::new();
    for path in &paths {
        let name = name_of(path);
        if dir.starts_with(path) {
            return Err(format!("{name} can't go inside itself."));
        }
        if taken(&dir.join(&name)) || !targets.insert(name.clone()) {
            return Err(format!("{name} already exists in {}.", name_of(dir)));
        }
    }
    Ok(paths
        .into_iter()
        .map(|path| {
            let to = dir.join(name_of(&path));
            (path, to)
        })
        .collect())
}

/// The byte range of `name` without its extension, which a rename field
/// starts with selected so typing keeps the extension. Dotfiles and
/// folders are taken whole.
pub fn stem_range(name: &str, is_dir: bool) -> Range<usize> {
    match name.rfind('.') {
        Some(ix) if ix > 0 && !is_dir => 0..ix,
        _ => 0..name.len(),
    }
}

/// What to ask before moving `rows` (each a name and whether it's a
/// folder) to the Trash: a title and a detail line. `None` for no rows.
pub fn trash_question(rows: &[(&str, bool)]) -> Option<(String, &'static str)> {
    Some(match rows {
        [] => return None,
        [(name, is_dir)] => (
            format!("Move {name} to the Trash?"),
            if *is_dir {
                "The folder and everything in it go to the Trash."
            } else {
                "It goes to the Trash."
            },
        ),
        rows => (
            format!("Move {} items to the Trash?", rows.len()),
            if rows.iter().any(|(_, is_dir)| *is_dir) {
                "They go to the Trash, folders with everything in them."
            } else {
                "They go to the Trash."
            },
        ),
    })
}
