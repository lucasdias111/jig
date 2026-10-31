//! The user's sketches on the Map: labelled boxes, labelled arrows, sticky
//! notes and frames, drawn at one level (Plan, Site or Domain) to think an
//! idea through, ask the agent about it and have it built. A sketch is
//! the user's own ink: the agent never edits it, only proposes beside it.
//!
//! Each is saved as you draw in `.jig/map/sketches/<title>.toml`, meant to
//! be committed, keyed by names, never by where things are on screen: an
//! arrow's or note's ends are its boxes' ids or real places
//! (`path#Type.method`, `path`, `folder/`, `domain:Name`), and a box is
//! placed relative to the nearest real box (`near`, `at`), so the sketch
//! moves with the code around it and merges stay readable.
//!
//! As the project is read again, a box is matched to real code by its
//! label, and turns solid; an arrow between two real ends turns solid
//! once the code calls along it. A match is made only when it's sure:
//! the label names one thing, or several in one file, or one near where
//! the box was drawn. Anything else stays sketched.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use super::index::{FnId, Project, Target};
use super::lens::{Place, slashed};

/// Where sketches are kept, in the project.
pub const DIR: &str = ".jig/map/sketches";

/// How a sketch's boxes are named for the agent: `sketch:<name>/<box>`.
pub const PREFIX: &str = "sketch:";

/// A sketch, and the file it's in.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Sketch {
    /// Its file's name without `.toml`, from its title.
    #[serde(skip)]
    pub name: String,
    pub title: String,
    /// The level it was drawn at: `domain`, `site`, or `plan:` and the
    /// place the Plan is of.
    pub level: String,
    /// The planner task made from it, by id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
    /// Done with: off the canvas, still in the Sketches list.
    #[serde(default, skip_serializing_if = "is_false")]
    pub archived: bool,
    #[serde(default, rename = "box", skip_serializing_if = "Vec::is_empty")]
    pub boxes: Vec<SketchBox>,
    #[serde(default, rename = "arrow", skip_serializing_if = "Vec::is_empty")]
    pub arrows: Vec<SketchArrow>,
    #[serde(default, rename = "note", skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Note>,
    #[serde(default, rename = "frame", skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<Frame>,
}

/// A box: something that should exist.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SketchBox {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub purpose: String,
    /// The real place it's placed by, and where it is from that box's top
    /// left, in the level's drawing.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub near: String,
    pub at: [f32; 2],
    pub size: [f32; 2],
}

/// An arrow between two of the sketch's boxes, or a box and a real place.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SketchArrow {
    /// A box's id or a real place.
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

/// A sticky note: on a box or a real place, or floating near one.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Note {
    pub id: String,
    pub text: String,
    /// What it's about, and moves with: a box's id or a real place.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub on: String,
    /// Floating: the real place it's placed by.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub near: String,
    pub at: [f32; 2],
    pub size: [f32; 2],
}

/// A frame around part of the sketch, with a label.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub near: String,
    pub at: [f32; 2],
    pub size: [f32; 2],
}

fn is_false(value: &bool) -> bool {
    !value
}

/// Which kind of shape, as selections name them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ShapeKind {
    Box,
    /// By its place in the arrows: arrows have no ids of their own.
    Arrow,
    Note,
    Frame,
}

/// A shape of a sketch: the sketch's name, its kind, and its id (an
/// arrow's place in the list).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ShapeRef {
    pub sketch: String,
    pub kind: ShapeKind,
    pub id: String,
}

impl ShapeRef {
    /// As the Map's selection keeps it: `sketch:<name>/box/<id>`.
    pub fn key(&self) -> String {
        let kind = match self.kind {
            ShapeKind::Box => "box",
            ShapeKind::Arrow => "arrow",
            ShapeKind::Note => "note",
            ShapeKind::Frame => "frame",
        };
        format!("{PREFIX}{}/{kind}/{}", self.sketch, self.id)
    }

    pub fn of_key(key: &str) -> Option<ShapeRef> {
        let rest = key.strip_prefix(PREFIX)?;
        let mut parts = rest.splitn(3, '/');
        let sketch = parts.next()?.to_string();
        let kind = match parts.next()? {
            "box" => ShapeKind::Box,
            "arrow" => ShapeKind::Arrow,
            "note" => ShapeKind::Note,
            "frame" => ShapeKind::Frame,
            _ => return None,
        };
        let id = parts.next()?.to_string();
        Some(ShapeRef { sketch, kind, id })
    }
}

impl Sketch {
    /// A new sketch at `level`, its name not yet taken in `taken`.
    pub fn new(title: &str, level: &str, taken: &[String]) -> Sketch {
        Sketch {
            name: unique(&slug(title, "sketch"), taken),
            title: title.to_string(),
            level: level.to_string(),
            ..Sketch::default()
        }
    }

    /// An id for a new shape from `label`, not used yet in the sketch.
    pub fn new_id(&self, label: &str, fallback: &str) -> String {
        let taken: Vec<String> = self
            .boxes
            .iter()
            .map(|b| b.id.clone())
            .chain(self.notes.iter().map(|n| n.id.clone()))
            .chain(self.frames.iter().map(|f| f.id.clone()))
            .collect();
        unique(&slug(label, fallback), &taken)
    }

    pub fn find_box(&self, id: &str) -> Option<&SketchBox> {
        self.boxes.iter().find(|b| b.id == id)
    }

    /// Whether `end` (of an arrow or a note) is one of its boxes.
    pub fn is_box(&self, end: &str) -> bool {
        self.find_box(end).is_some()
    }

    /// Every real place it names: what it's placed by and attached to.
    pub fn places(&self) -> Vec<&str> {
        let mut places: Vec<&str> = Vec::new();
        let ends = self
            .arrows
            .iter()
            .flat_map(|a| [a.from.as_str(), a.to.as_str()])
            .chain(self.notes.iter().map(|n| n.on.as_str()))
            .filter(|end| !self.is_box(end));
        let nears = self
            .boxes
            .iter()
            .map(|b| b.near.as_str())
            .chain(self.notes.iter().map(|n| n.near.as_str()))
            .chain(self.frames.iter().map(|f| f.near.as_str()));
        for place in ends.chain(nears) {
            if !place.is_empty() && !places.contains(&place) {
                places.push(place);
            }
        }
        places
    }

    /// The real places it's attached to: arrows' and notes' ends.
    pub fn attached(&self) -> Vec<&str> {
        let mut places: Vec<&str> = Vec::new();
        let ends = self
            .arrows
            .iter()
            .flat_map(|a| [a.from.as_str(), a.to.as_str()])
            .chain(self.notes.iter().map(|n| n.on.as_str()));
        for end in ends {
            if !end.is_empty() && !self.is_box(end) && !places.contains(&end) {
                places.push(end);
            }
        }
        places
    }

    /// Whether it holds nothing.
    pub fn is_empty(&self) -> bool {
        self.boxes.is_empty()
            && self.arrows.is_empty()
            && self.notes.is_empty()
            && self.frames.is_empty()
    }

    /// Box `id` labelled `label`, its id following the label so the file
    /// reads by names: its new id, if it changed.
    pub fn relabel_box(&mut self, id: &str, label: &str) -> Option<String> {
        let at = self.boxes.iter().position(|b| b.id == id)?;
        self.boxes[at].label = label.to_string();
        let others: Vec<String> = self
            .boxes
            .iter()
            .map(|b| b.id.clone())
            .chain(self.notes.iter().map(|n| n.id.clone()))
            .chain(self.frames.iter().map(|f| f.id.clone()))
            .filter(|other| other != id)
            .collect();
        let new = unique(&slug(label, "box"), &others);
        if new == id {
            return None;
        }
        self.boxes[at].id = new.clone();
        for arrow in &mut self.arrows {
            for end in [&mut arrow.from, &mut arrow.to] {
                if end == id {
                    *end = new.clone();
                }
            }
        }
        for note in &mut self.notes {
            if note.on == id {
                note.on = new.clone();
            }
        }
        Some(new)
    }

    /// Box `id` taken out, with the arrows and notes on it.
    pub fn remove_box(&mut self, id: &str) {
        self.boxes.retain(|b| b.id != id);
        self.arrows.retain(|a| a.from != id && a.to != id);
        self.notes.retain(|n| n.on != id);
    }
}

/// How a sketch box's own name for the agent reads: `sketch:<name>/<box>`.
pub fn place_name(sketch: &str, id: &str) -> String {
    format!("{PREFIX}{sketch}/{id}")
}

/// The sketch and box a `sketch:<name>/<box>` name is of.
pub fn parse_place(name: &str) -> Option<(String, String)> {
    let rest = name.trim().strip_prefix(PREFIX)?;
    let (sketch, id) = rest.split_once('/')?;
    // Selection keys (`<name>/box/<id>`) name boxes too.
    let id = id.strip_prefix("box/").unwrap_or(id);
    (!sketch.is_empty() && !id.is_empty()).then(|| (sketch.to_string(), id.to_string()))
}

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

/// The file sketch `name` is in, relative to the project, as a task links
/// to it.
pub fn link(name: &str) -> String {
    format!("{DIR}/{name}.toml")
}

/// The sketch a link of a task's names, if it's to one.
pub fn of_link(link: &str) -> Option<String> {
    let link = link.trim().trim_start_matches("./");
    let rest = link.strip_prefix(DIR)?.strip_prefix('/')?;
    let name = rest.strip_suffix(".toml")?;
    (!name.is_empty() && !name.contains('/')).then(|| name.to_string())
}

/// The sketches a task's description links to, by name.
pub fn linked_in(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(DIR) {
        let tail = &rest[at..];
        let end = tail
            .find(".toml")
            .map_or(tail.len(), |end| end + ".toml".len());
        if let Some(name) = of_link(&tail[..end])
            && !names.contains(&name)
        {
            names.push(name);
        }
        rest = &tail[end.max(1)..];
    }
    names
}

/// A name from a title: lower case words joined by dashes.
pub fn slug(title: &str, fallback: &str) -> String {
    let words: Vec<String> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .take(8)
        .map(str::to_lowercase)
        .collect();
    match words.join("-") {
        name if name.is_empty() => fallback.into(),
        name => name,
    }
}

/// `base`, or `base-2` and on, whichever isn't in `taken`.
pub fn unique(base: &str, taken: &[String]) -> String {
    if !taken.iter().any(|t| t == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|name| !taken.contains(name))
        .expect("some number is free")
}

/// What a box's label matches in the code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Match {
    /// Nothing yet: still an idea.
    None,
    /// This place, for sure.
    Real(String),
    /// Several things it could be: left sketched.
    Unsure(usize),
}

impl Match {
    pub fn place(&self) -> Option<&str> {
        match self {
            Match::Real(place) => Some(place),
            _ => None,
        }
    }
}

/// A label as names are compared: letters and digits, lower case, so
/// `Rate limiter`, `RateLimiter` and `rate_limiter.rs` are the same.
fn normal(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// A file's name without its extension.
fn stem(path: &str) -> &str {
    let name = path.rsplit('/').next().unwrap_or(path);
    name.split('.').next().unwrap_or(name)
}

/// What box `label` is in `project`, placed near `near` (the folder or
/// file of the real box it was drawn by): a type, a function or a file of
/// that name. Several in one file are one thing (a type, its file and its
/// constructor); several in different files are narrowed to those near
/// the box, and if that doesn't leave one, it's unsure.
pub fn match_box(project: &Project, label: &str, near: Option<&str>) -> Match {
    let wanted = normal(label.split('(').next().unwrap_or(label));
    if wanted.chars().count() < 3 {
        return Match::None;
    }
    // Candidates by file: the best name in each (a type over a function
    // over the file itself).
    let mut found: Vec<(String, u8, String)> = Vec::new();
    let mut add =
        |file: String, rank: u8, place: String| match found.iter_mut().find(|(f, _, _)| *f == file)
        {
            Some(entry) if entry.1 < rank => *entry = (file, rank, place),
            Some(_) => {}
            None => found.push((file, rank, place)),
        };
    for file in &project.files {
        let path = slashed(&file.path);
        for ty in &file.types {
            if normal(&ty.name) == wanted {
                add(path.clone(), 3, format!("{path}#{}", ty.name));
            }
        }
        if normal(stem(&path)) == wanted {
            add(path.clone(), 1, path.clone());
        }
    }
    for function in &project.functions {
        if normal(function.short()) == wanted || normal(&function.title()) == wanted {
            let path = slashed(&project.files[function.file].path);
            add(path.clone(), 2, format!("{path}#{}", function.title()));
        }
    }
    for other in &project.others {
        let path = slashed(other);
        if normal(stem(&path)) == wanted {
            add(path.clone(), 1, path);
        }
    }
    match found.len() {
        0 => Match::None,
        1 => Match::Real(found.remove(0).2),
        n => {
            let Some(near) = near.map(folder_of).filter(|dir| !dir.is_empty()) else {
                return Match::Unsure(n);
            };
            // In the box's own folder first, then anywhere under it.
            let here: Vec<&(String, u8, String)> = found
                .iter()
                .filter(|(file, _, _)| folder_of(file) == near)
                .collect();
            let under: Vec<&(String, u8, String)> = found
                .iter()
                .filter(|(file, _, _)| file.starts_with(&near))
                .collect();
            match (here.as_slice(), under.as_slice()) {
                ([one], _) | ([], [one]) => Match::Real(one.2.clone()),
                _ => Match::Unsure(n),
            }
        }
    }
}

/// The folder a place is in (`a/b/` for `a/b/c.rs#T`, itself for a folder).
fn folder_of(place: &str) -> String {
    let path = place.split('#').next().unwrap_or(place);
    if path.ends_with('/') {
        return path.to_string();
    }
    match path.rfind('/') {
        Some(at) => path[..=at].to_string(),
        None => String::new(),
    }
}

/// The functions place `place` holds: itself, a type's, a file's or a
/// folder's.
pub fn functions_in(project: &Project, place: &Place) -> Vec<FnId> {
    if let Some(id) = place.function_in(project) {
        return vec![id];
    }
    let in_file = |file: usize, owner: Option<&str>| -> Vec<FnId> {
        project
            .functions
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                f.file == file
                    && owner.is_none_or(|owner| f.outline.owner.as_deref() == Some(owner))
            })
            .map(|(id, _)| id)
            .collect()
    };
    match place {
        Place::File { path, owner, .. } => project
            .files
            .iter()
            .position(|f| slashed(&f.path) == *path)
            .map(|file| in_file(file, owner.as_deref()))
            .unwrap_or_default(),
        Place::Folder(dir) => project
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| slashed(&f.path).starts_with(dir.as_str()))
            .flat_map(|(file, _)| in_file(file, None))
            .collect(),
        Place::Domain(_) | Place::Sketch { .. } => Vec::new(),
    }
}

/// Whether anything in `from` surely calls anything in `to`.
pub fn calls_between(project: &Project, from: &[FnId], to: &[FnId]) -> bool {
    let to: HashSet<FnId> = to.iter().copied().collect();
    from.iter().any(|&id| {
        project.edges.get(id).is_some_and(|edges| {
            edges.iter().any(|edge| match &edge.target {
                Target::Function(target) => to.contains(target),
                // A guess by name alone isn't a call to snap to.
                _ => false,
            })
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::tests::fixture;

    fn sketch() -> Sketch {
        Sketch {
            name: "rate-limiting".into(),
            title: "Rate limiting".into(),
            level: "site".into(),
            task: None,
            archived: false,
            boxes: vec![SketchBox {
                id: "rate-limiter".into(),
                label: "RateLimiter".into(),
                purpose: "Keeps each user's budget".into(),
                near: "src/api/".into(),
                at: [40., 120.],
                size: [180., 64.],
            }],
            arrows: vec![SketchArrow {
                from: "src/api/handlers.rs".into(),
                to: "rate-limiter".into(),
                label: "checks".into(),
            }],
            notes: vec![Note {
                id: "note".into(),
                text: "Per user, not per address".into(),
                on: "rate-limiter".into(),
                near: String::new(),
                at: [10., -40.],
                size: [160., 70.],
            }],
            frames: Vec::new(),
        }
    }

    #[test]
    fn sketches_are_saved_as_readable_toml_and_read_back() {
        let root = tempfile::tempdir().unwrap();
        let sketch = sketch();
        save(root.path(), &sketch).unwrap();
        let text = std::fs::read_to_string(file(root.path(), "rate-limiting")).unwrap();
        assert!(text.contains("level = \"site\""), "{text}");
        assert!(text.contains("[[box]]"), "{text}");
        assert!(text.contains("near = \"src/api/\""), "{text}");
        assert!(text.contains("to = \"rate-limiter\""), "{text}");
        assert!(!text.contains("archived"), "{text}");
        assert_eq!(read_all(root.path()), std::slice::from_ref(&sketch));
        assert_eq!(sketch.attached(), ["src/api/handlers.rs"]);
        assert_eq!(sketch.places(), ["src/api/handlers.rs", "src/api/"]);
        remove(root.path(), "rate-limiting").unwrap();
        assert!(read_all(root.path()).is_empty());
    }

    #[test]
    fn names_and_keys_go_both_ways() {
        let shape = ShapeRef {
            sketch: "limits".into(),
            kind: ShapeKind::Note,
            id: "note-2".into(),
        };
        assert_eq!(ShapeRef::of_key(&shape.key()), Some(shape));
        assert_eq!(
            parse_place("sketch:limits/rate-limiter"),
            Some(("limits".into(), "rate-limiter".into()))
        );
        assert_eq!(of_link(&link("limits")), Some("limits".into()));
        assert_eq!(of_link("src/a.toml"), None);
        assert_eq!(
            linked_in(
                "Sketch: [Limits](.jig/map/sketches/limits.toml) and [x](.jig/map/sketches/x.toml)"
            ),
            ["limits", "x"]
        );
        let taken = vec!["box".to_string(), "box-2".to_string()];
        assert_eq!(unique("box", &taken), "box-3");
        assert_eq!(slug("Rate limiting!", "x"), "rate-limiting");
    }

    #[test]
    fn a_box_is_matched_only_when_its_sure() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let users = "server/src/org/demo/users/UserService.java";
        assert_eq!(
            match_box(&project, "User service", None),
            Match::Real(format!("{users}#UserService"))
        );
        assert_eq!(match_box(&project, "Nothing like it", None), Match::None);
        assert_eq!(match_box(&project, "ab", None), Match::None, "too short");
    }

    #[test]
    fn several_of_one_name_are_narrowed_by_where_the_box_is() {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write("a/limit.rs", "pub struct Limiter;\n");
        write("b/limit.rs", "pub struct Limiter;\n");
        write("b/c/limit.rs", "pub struct Limiter;\n");
        let project = Project::read(dir.path());
        assert_eq!(match_box(&project, "Limiter", None), Match::Unsure(3));
        assert_eq!(
            match_box(&project, "Limiter", Some("b/")),
            Match::Real("b/limit.rs#Limiter".into()),
            "its own folder's over one deeper"
        );
        assert_eq!(
            match_box(&project, "Limiter", Some("b/c/")),
            Match::Real("b/c/limit.rs#Limiter".into())
        );
    }
}
