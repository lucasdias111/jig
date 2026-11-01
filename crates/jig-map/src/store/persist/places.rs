//! How each Map view is arranged, in `.jig/map/places.json`: one person's
//! arrangement, so kept out of git.

use std::path::{Path, PathBuf};

use serde_json::Value;

/// Where views keep how they were arranged, in the project at `root`.
pub fn file(root: &Path) -> PathBuf {
    root.join(".jig/map/places.json")
}

pub fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

pub fn write_json(path: &Path, value: &Value) {
    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    let _ = std::fs::write(path, text + "\n");
}

/// `.jig/.gitignore` keeps the Map's places out of git: they're how one
/// person arranged things, not the project's.
pub fn keep_out_of_git(root: &Path) {
    let file = root.join(".jig/.gitignore");
    let line = "map/places.json";
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    if text.lines().any(|l| l.trim() == line) {
        return;
    }
    let mut text = text;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
    let _ = std::fs::write(file, text);
}
