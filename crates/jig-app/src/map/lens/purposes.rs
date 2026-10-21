//! Purposes and arrow labels the agent described (`map_describe`) and the
//! user kept, in `.jig/map/purposes.toml`, committed: what a box at Site
//! is for, by its place, over what its README or doc comment says; and
//! what goes along an arrow between two places. A domain's purpose and the
//! words on a link between domains live in `domains.toml` instead.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

pub const FILE: &str = ".jig/map/purposes.toml";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Purposes {
    /// By place: `dir/`, `path` or `path#Type`.
    #[serde(default, rename = "purpose")]
    pub purposes: BTreeMap<String, String>,
    #[serde(default, rename = "label", skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<Label>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub from: String,
    pub to: String,
    pub label: String,
}

impl Purposes {
    /// What goes from `from` to `to`, in words.
    pub fn set_label(&mut self, from: &str, to: &str, label: &str) {
        let label = label.trim().to_string();
        match self
            .labels
            .iter_mut()
            .find(|l| l.from == from && l.to == to)
        {
            Some(found) => found.label = label,
            None => self.labels.push(Label {
                from: from.to_string(),
                to: to.to_string(),
                label,
            }),
        }
    }
}

/// The purposes kept in the project at `root`; none when there's no file.
pub fn read(root: &Path) -> Purposes {
    std::fs::read_to_string(root.join(FILE))
        .ok()
        .and_then(|text| toml::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn write(root: &Path, purposes: &Purposes) -> Result<()> {
    let path = root.join(FILE);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    std::fs::write(&path, toml::to_string(purposes)?).context("Couldn't save the purposes")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purposes_and_labels_are_kept_by_place() {
        let root = tempfile::tempdir().unwrap();
        let mut purposes = read(root.path());
        purposes
            .purposes
            .insert("src/map/".into(), "Draws the project".into());
        purposes.set_label("src/a.rs", "src/b.rs", "orders");
        purposes.set_label("src/a.rs", "src/b.rs", "bookings");
        write(root.path(), &purposes).unwrap();
        let read_back = read(root.path());
        assert_eq!(read_back, purposes);
        assert_eq!(read_back.labels.len(), 1);
        assert_eq!(read_back.labels[0].label, "bookings");
    }
}
