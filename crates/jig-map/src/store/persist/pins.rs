//! Pinned drawings: a drawing worth keeping past its conversation, saved
//! in `.jig/map/pins/<title>.toml` and committed, keyed by the places it
//! names, never by positions, so merges stay readable. The Map shows them
//! on a Pinned layer of their own.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::domain::lens::Drawing;

/// Where pins are kept, in the project.
pub const DIR: &str = ".jig/map/pins";

/// A drawing pinned, and the file it's in (its name without `.toml`).
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub name: String,
    pub drawing: Drawing,
}

/// Every pin in the project at `root`, by name.
pub fn read_all(root: &Path) -> Vec<Pin> {
    let Ok(entries) = std::fs::read_dir(root.join(DIR)) else {
        return Vec::new();
    };
    let mut pins: Vec<Pin> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let drawing = toml::from_str(&text).ok()?;
            Some(Pin {
                name: path.file_stem()?.to_string_lossy().into_owned(),
                drawing,
            })
        })
        .collect();
    pins.sort_by(|a, b| a.name.cmp(&b.name));
    pins
}

/// Pin `drawing`: saved under its title, or a title like it if that one's
/// taken by another drawing. Its name back.
pub fn save(root: &Path, drawing: &Drawing) -> Result<String> {
    let dir = root.join(DIR);
    std::fs::create_dir_all(&dir).context("Couldn't make the pins folder")?;
    let base = slug(&drawing.summary());
    let text = toml::to_string(drawing)?;
    let mut name = base.clone();
    let mut n = 2;
    loop {
        let path = file(root, &name);
        match std::fs::read_to_string(&path) {
            Ok(old) if old == text => return Ok(name),
            Ok(_) => {
                name = format!("{base}-{n}");
                n += 1;
            }
            Err(_) => {
                std::fs::write(&path, text).context("Couldn't save the pin")?;
                return Ok(name);
            }
        }
    }
}

/// Unpin the pin named `name`.
pub fn remove(root: &Path, name: &str) -> Result<()> {
    std::fs::remove_file(file(root, name)).context("Couldn't remove the pin")
}

fn file(root: &Path, name: &str) -> PathBuf {
    root.join(DIR).join(format!("{name}.toml"))
}

/// A file name from a title: lower case words joined by dashes.
fn slug(title: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(8)
        .map(str::to_lowercase)
        .collect();
    match words.join("-") {
        name if name.is_empty() => "pin".into(),
        name => name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::lens::{Kind, Step};

    #[test]
    fn pins_are_saved_by_title_and_read_back() {
        let root = tempfile::tempdir().unwrap();
        let drawing = Drawing {
            kind: Kind::Trace,
            title: "Booking a class".into(),
            steps: vec![Step {
                place: "a.rs#book".into(),
                label: "Book".into(),
            }],
            links: Vec::new(),
            ghosts: Vec::new(),
        };
        let name = save(root.path(), &drawing).unwrap();
        assert_eq!(name, "trace-booking-a-class-1-step");
        assert_eq!(save(root.path(), &drawing).unwrap(), name, "the same pin");
        let text =
            std::fs::read_to_string(root.path().join(DIR).join(format!("{name}.toml"))).unwrap();
        assert!(text.contains("kind = \"trace\""), "{text}");
        assert!(text.contains("[[step]]"), "{text}");
        let other = Drawing {
            kind: Kind::Highlight,
            ..drawing.clone()
        };
        let mut renamed = other.clone();
        renamed.kind = Kind::Trace;
        renamed.steps[0].label = "Other".into();
        assert_eq!(save(root.path(), &renamed).unwrap(), format!("{name}-2"));
        let pins = read_all(root.path());
        assert_eq!(pins.len(), 2);
        assert_eq!(pins[0].drawing, drawing);
        remove(root.path(), &name).unwrap();
        assert_eq!(read_all(root.path()).len(), 1);
    }
}
