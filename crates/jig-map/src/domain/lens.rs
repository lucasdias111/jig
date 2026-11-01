//! What the agent draws on the Map: the Lens. Each call of one of the
//! Map's tools (`jig_ai::map`) is one drawing: a highlight, a note, a
//! trace or tour, a group, ghosts it proposes, or purposes and labels it
//! describes. The agent names places as `path#Type.method`,
//! `path#function`, `path`, `folder/` or `domain:Name`; each is resolved
//! against the project's index, accepting any real file, and a name that
//! isn't found comes back with names like it. `mcp` serves the tools; the
//! app reads each call back from the agent's own report of it, so a
//! drawing belongs to its conversation and comes back with it.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::domains::Domains;
use super::index::{FnId, Project};
mod calls;
mod resolving;
pub use calls::*;
pub use resolving::*;

/// What a drawing is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Places lit up.
    Highlight,
    /// A remark on one place.
    Note,
    /// A path through places, in order, each step labelled.
    Trace,
    /// A trace the camera flies through.
    Tour,
    /// Places circled together, with a label.
    Group,
    /// Ghost boxes and arrows: a change the agent suggests.
    Propose,
    /// Purposes for places and labels for arrows, to keep or not.
    Describe,
}

impl Kind {
    /// Its steps are in order, to step through.
    pub fn stepped(self) -> bool {
        matches!(self, Kind::Trace | Kind::Tour)
    }

    pub fn name(self) -> &'static str {
        match self {
            Kind::Highlight => "Highlight",
            Kind::Note => "Note",
            Kind::Trace => "Trace",
            Kind::Tour => "Tour",
            Kind::Group => "Group",
            Kind::Propose => "Proposal",
            Kind::Describe => "Descriptions",
        }
    }
}

/// A place the agent named, with what it says there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    /// As the agent named it.
    pub place: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

/// An arrow between two places, or a place and a ghost.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub from: String,
    pub to: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
}

/// A box the agent proposes, drawn as a ghost near a real place.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ghost {
    pub name: String,
    /// The real place it goes by.
    pub near: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub purpose: String,
}

/// One call of the Map's tools.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drawing {
    pub kind: Kind,
    #[serde(default)]
    pub title: String,
    /// The places, in order: lit, noted, traced, grouped or described.
    #[serde(default, rename = "step", skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
    /// Arrows lit, proposed or labelled.
    #[serde(default, rename = "link", skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    #[serde(default, rename = "ghost", skip_serializing_if = "Vec::is_empty")]
    pub ghosts: Vec<Ghost>,
}

impl Drawing {
    /// A line for the transcript: "Trace: Booking a class · 10 steps".
    pub fn summary(&self) -> String {
        let count = |n: usize, what: &str| match (n, what) {
            (1, _) => format!("1 {what}"),
            (n, "box") => format!("{n} boxes"),
            (n, _) => format!("{n} {what}s"),
        };
        let what = match self.kind {
            Kind::Trace | Kind::Tour => count(self.steps.len(), "step"),
            Kind::Propose => count(self.ghosts.len(), "box"),
            Kind::Describe => count(self.steps.len() + self.links.len(), "description"),
            Kind::Note => self
                .steps
                .first()
                .map_or(String::new(), |s| s.place.clone()),
            _ => count(self.steps.len(), "place"),
        };
        match (self.title.trim(), what.is_empty()) {
            ("", _) => format!("{}: {what}", self.kind.name()),
            (title, true) => format!("{}: {title}", self.kind.name()),
            (title, false) => format!("{}: {title} · {what}", self.kind.name()),
        }
    }

    /// Every place it names, ghosts' names left out.
    pub fn places(&self) -> Vec<&str> {
        let ghost = |name: &str| self.ghosts.iter().any(|g| g.name == name);
        let mut places: Vec<&str> = self.steps.iter().map(|s| s.place.as_str()).collect();
        for link in &self.links {
            for end in [&link.from, &link.to] {
                if !ghost(end) {
                    places.push(end);
                }
            }
        }
        places.extend(self.ghosts.iter().map(|g| g.near.as_str()));
        places
    }
}

/// What a call of the Map's tools does.
#[derive(Clone, Debug, PartialEq)]
pub enum Call {
    Draw(Drawing),
    /// Everything drawn so far in the conversation goes.
    Clear,
}

/// Where a named place is in the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// A domain, by its name.
    Domain(String),
    /// A folder, `dir/`.
    Folder(String),
    /// A file, read by the index or not, maybe a type in it, maybe a
    /// function (`Owner.name` or `name`).
    File {
        path: String,
        owner: Option<String>,
        function: Option<String>,
    },
    /// A box of the user's sketch (`sketch:<name>/<box>`), by their ids.
    Sketch { sketch: String, shape: String },
}

impl Place {
    /// As Site names its boxes: `dir/`, `path` or `path#Type`.
    pub fn site_place(&self) -> Option<String> {
        match self {
            Place::Domain(_) | Place::Sketch { .. } => None,
            Place::Folder(dir) => Some(dir.clone()),
            Place::File {
                path,
                owner: Some(owner),
                ..
            } => Some(format!("{path}#{owner}")),
            Place::File { path, .. } => Some(path.clone()),
        }
    }

    /// The file it's in, if it's in one.
    pub fn file(&self) -> Option<&str> {
        match self {
            Place::File { path, .. } => Some(path),
            _ => None,
        }
    }

    /// The function it is, in `project`.
    pub fn function_in(&self, project: &Project) -> Option<FnId> {
        let Place::File {
            path,
            function: Some(title),
            ..
        } = self
        else {
            return None;
        };
        let file = project
            .files
            .iter()
            .position(|file| slashed(&file.path) == *path)?;
        project
            .functions
            .iter()
            .position(|f| f.file == file && f.title() == *title)
    }
}

/// The user's sketches as saved, which places can name. The store reads
/// them.
pub trait SavedSketches {
    fn sketches(&self) -> Vec<super::sketch::Sketch>;
}

/// A path with `/`, as places are named.
pub fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
pub mod tests;
