//! Making, renaming, moving and trashing files and folders on disk, after
//! [`paths`](crate::domain::paths) has checked the names.

use std::fs;
use std::path::{Path, PathBuf};

use crate::domain::paths::{self, Moved, NewEntry};

/// Create `name` inside `dir` and return its path. `name` may contain `/`
/// to create folders on the way, like `src/ui/tree.rs`.
pub fn create_entry(dir: &Path, name: &str, kind: NewEntry) -> Result<PathBuf, String> {
    let relative = paths::new_name(name)?;
    let name = name.trim();
    let path = dir.join(relative);
    if path.exists() {
        return Err(format!("{name} already exists."));
    }
    let created = match kind {
        NewEntry::Folder => fs::create_dir_all(&path),
        NewEntry::File => path
            .parent()
            .map_or(Ok(()), fs::create_dir_all)
            .and_then(|()| {
                fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map(drop)
            }),
    };
    created.map_err(|error| format!("Could not create {name}: {error}"))?;
    Ok(path)
}

/// Give `path` the name `name` in the same folder and return its new path.
/// A change of case alone is allowed, as Finder allows it.
pub fn rename_entry(path: &Path, name: &str) -> Result<PathBuf, String> {
    let name = paths::rename_name(name)?;
    let Some(dir) = path.parent() else {
        return Err("This can't be renamed.".into());
    };
    let new = dir.join(name);
    if new == path {
        return Ok(new);
    }
    if new.exists() && !paths::same_but_case(path, name) {
        return Err(format!("{name} already exists."));
    }
    fs::rename(path, &new).map_err(|error| format!("Could not rename to {name}: {error}"))?;
    Ok(new)
}

/// Move each of `paths` into `dir`, keeping its name. Everything is checked
/// first, so a name already taken or a folder dropped into one inside it
/// moves nothing; entries already in `dir`, or `dir` itself, stay put.
pub fn move_entries(paths: &[PathBuf], dir: &Path) -> Moved {
    let planned = match paths::plan_move(paths, dir, Path::exists) {
        Ok(planned) => planned,
        Err(error) => {
            return Moved {
                moves: Vec::new(),
                error: Some(error),
            };
        }
    };
    let mut moved = Moved::default();
    for (path, to) in planned {
        if let Err(error) = fs::rename(&path, &to) {
            moved.error = Some(format!("Could not move {}: {error}", paths::name_of(&path)));
            break;
        }
        moved.moves.push((path, to));
    }
    moved
}

/// Move `paths` to the Trash. A failure part way may have trashed some
/// already.
pub fn move_to_trash(paths: &[PathBuf]) -> Result<(), String> {
    trash::delete_all(paths).map_err(|error| error.to_string())
}
