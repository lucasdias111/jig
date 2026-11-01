//! The user's sketches as saved in `.jig/map/sketches/<title>.toml`,
//! committed.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::domain::lens::SavedSketches;
use crate::domain::sketch::{DIR, Sketch};
use crate::store::cache::Folder;

/// Every sketch in the project at `root`, by name.
pub fn read_all(root: &Path) -> Vec<Sketch> {
    let Ok(entries) = std::fs::read_dir(root.join(DIR)) else {
        return Vec::new();
    };
    let mut sketches: Vec<Sketch> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let mut sketch: Sketch = toml::from_str(&text).ok()?;
            sketch.name = path.file_stem()?.to_string_lossy().into_owned();
            Some(sketch)
        })
        .collect();
    sketches.sort_by(|a, b| a.name.cmp(&b.name));
    sketches
}

/// Write `sketch` to its file, unless it already says that.
pub fn save(root: &Path, sketch: &Sketch) -> Result<()> {
    let dir = root.join(DIR);
    std::fs::create_dir_all(&dir).context("Couldn't make the sketches folder")?;
    let text = toml::to_string(sketch).context("Couldn't write the sketch")?;
    let path = file(root, &sketch.name);
    if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
        return Ok(());
    }
    std::fs::write(&path, text).context("Couldn't save the sketch")
}

/// Remove the sketch named `name`'s file.
pub fn remove(root: &Path, name: &str) -> Result<()> {
    match std::fs::remove_file(file(root, name)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            Err(error).context("Couldn't remove the sketch")
        }
        _ => Ok(()),
    }
}

/// The file sketch `name` is in.
pub fn file(root: &Path, name: &str) -> PathBuf {
    root.join(DIR).join(format!("{name}.toml"))
}

impl SavedSketches for Folder<'_> {
    fn sketches(&self) -> Vec<Sketch> {
        read_all(self.0)
    }
}
