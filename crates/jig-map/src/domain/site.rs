//! The Site level: the project as Domain draws it, one zoom further in.
//! Each domain (and Shared and Unsorted) is a region holding only its own
//! share of the code: its components (packages, or top folders), then
//! their modules and folders, then files, and a file's types where it has
//! several. The same component shows in every domain with code in it,
//! each region with its own slice.
//!
//! Folders that only lead somewhere are folded away: a chain of single
//! folders, `src`, `main` and `java`, and a component holding one folder
//! is one box (`balladeby-backend › bookings`), as is a folder holding one
//! file. Tests are left out unless asked for.
//!
//! Everything is laid out once, every box at the size its contents take,
//! so opening one as you zoom in only shows what's inside it: nothing
//! moves. A box opens once what's inside would read on screen. Calls are
//! counted between files and types, and drawn only between boxes side by
//! side inside a region; between regions there's only the domains' one
//! link per pair, as at Domain.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use super::domains::{Domains, Home, all_files, place_of};
use super::index::{Project, Target};
use super::layout::Rect;
use super::plan::slashed;
mod arrows;
mod building;
mod labels;
mod sizing;
mod units;
use arrows::*;
use labels::*;
use units::*;

/// A file box, closed or without types, at its narrowest and widest.
const FILE_WIDTH: f32 = 190.;
const FILE_MAX_WIDTH: f32 = 420.;
const FILE_HEIGHT: f32 = 74.;
const TYPE_WIDTH: f32 = 170.;
const TYPE_HEIGHT: f32 = 56.;
/// A frame's title bar at least, and the room round what's in it.
pub const TITLE: f32 = 34.;
const PAD: f32 = 14.;
const GAP: f32 = 14.;
/// Room at a region's top for its name, purpose and offers, at least.
pub const REGION_HEADER: f32 = 190.;
const REGION_PAD: f32 = 22.;
const REGION_MIN_WIDTH: f32 = 720.;
/// Between regions: room for the links' labels and the names above.
pub const REGION_GAP: f32 = 300.;
/// Between the domains and Shared and Unsorted below them.
const APART: f32 = 300.;
/// How much wider than tall contents are packed, and the regions.
const ASPECT: f32 = 1.6;
const REGIONS_ASPECT: f32 = 2.4;
/// Examples kept of the calls behind an arrow, for its tooltip.
const MAX_EXAMPLES: usize = 12;
/// A box starts opening this wide on screen, and once a typical box
/// inside would be this wide (or it's `OPEN_BY` wide); it's open once it's `OPEN_SPAN` times that.
const OPEN_FROM: f32 = 200.;
const READ: f32 = 120.;
const REGION_OPEN: f32 = 380.;
/// However small what's inside, a box this wide on screen is opening.
const OPEN_BY: f32 = 700.;
pub const OPEN_SPAN: f32 = 1.3;
/// Folders that only hold the code, never say what it is.
const BOILERPLATE: [&str; 7] = [
    "src", "main", "java", "kotlin", "scala", "source", "sources",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The project itself, holding the regions.
    Root,
    /// A domain, Shared or Unsorted.
    Region(Home),
    /// A package, or a top folder.
    Component,
    Folder,
    /// A file, read by the outline or only placed by its folder.
    File {
        read: bool,
    },
    /// A type declared in a file.
    Type,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// What keeps it in saved places and selections: `domain:<name>`,
    /// `dir:<region>:a/b/`, `file:a/b.rs`, `type:a/b.rs#B`.
    pub key: String,
    pub kind: Kind,
    pub title: String,
    pub purpose: Option<String>,
    /// How much it holds: `12 files · 340 functions`.
    pub detail: String,
    /// What's inside, by name: `controllers · dto · services`.
    pub contents: String,
    /// As the Map names places: `a/b/`, `a/b.rs`, `a/b.rs#B`.
    pub place: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Where it is, in unzoomed points; a folder's box is its frame.
    pub rect: Rect,
    /// Its title bar, or a region's header, in unzoomed points.
    pub header: f32,
    /// How many functions it holds, all the way down.
    pub functions: usize,
    /// The zoom it starts opening at; leaves never do.
    pub open_at: f32,
    /// The region it's in (a region's is itself, the project's 0).
    pub region: usize,
}

impl Node {
    /// Whether it opens to show what's inside.
    pub fn opens(&self) -> bool {
        !self.children.is_empty()
    }

    /// How far it has opened at `zoom`, from 0 to 1.
    pub fn opening(&self, zoom: f32) -> f32 {
        if self.kind == Kind::Root {
            return 1.;
        }
        if !self.opens() {
            return 0.;
        }
        let t = ((zoom / self.open_at - 1.) / (OPEN_SPAN - 1.)).clamp(0., 1.);
        t * t * (3. - 2. * t)
    }

    /// The zoom it's open at.
    pub fn open_to(&self) -> f32 {
        self.open_at * OPEN_SPAN
    }
}

/// The calls from one box to another, counted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Calls {
    pub count: usize,
    /// Some of them, as `caller → callee`.
    pub examples: Vec<String>,
}

impl Calls {
    fn add(&mut self, other: &Calls) {
        self.count += other.count;
        for example in &other.examples {
            if self.examples.len() < MAX_EXAMPLES && !self.examples.contains(example) {
                self.examples.push(example.clone());
            }
        }
    }
}

/// What Site shows beyond the domains' own code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Show {
    pub tests: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Site {
    /// In order, each before what's in it; the project's first.
    pub nodes: Vec<Node>,
    /// The calls between files and types, by (from, to) node.
    pub calls: HashMap<(usize, usize), Calls>,
    pub show: Show,
    by_key: HashMap<String, usize>,
}

/// An arrow between two boxes side by side: one per direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrow {
    pub from: usize,
    pub to: usize,
    pub calls: Calls,
}

/// A file, or a type with a home of its own, placed in its region.
struct Unit {
    path: PathBuf,
    place: String,
    home: Home,
    /// The index's file, if it read it.
    file: Option<usize>,
    /// A type with a home apart from its file's.
    own_type: Option<String>,
    purpose: Option<String>,
    functions: usize,
    /// The file's types that live with it: name, kind, purpose, functions.
    types: Vec<(String, &'static str, Option<String>, usize)>,
}

/// A folder of one region's units, as it's found.
#[derive(Default)]
struct Dir {
    dirs: BTreeMap<String, Dir>,
    units: Vec<usize>,
}

impl Dir {
    fn insert(&mut self, folders: &[String], unit: usize) {
        match folders.split_first() {
            None => self.units.push(unit),
            Some((first, rest)) => self
                .dirs
                .entry(first.clone())
                .or_default()
                .insert(rest, unit),
        }
    }
}

impl Site {
    pub fn index_of(&self, key: &str) -> Option<usize> {
        self.by_key.get(key).copied()
    }

    /// The regions, by node.
    pub fn regions(&self) -> impl Iterator<Item = usize> + '_ {
        self.nodes[0].children.iter().copied()
    }

    /// The box holding `place` (a folder `dir/`, a file or `path#Type`):
    /// itself, else the file or folder it's in.
    pub fn box_of(&self, place: &str) -> Option<usize> {
        let (path, owner) = match place.split_once('#') {
            Some((path, owner)) => (path, Some(owner)),
            None => (place, None),
        };
        if let Some(owner) = owner
            && let Some(ix) = self.index_of(&format!("type:{path}#{owner}"))
        {
            return Some(ix);
        }
        if let Some(dir) = path.strip_suffix('/') {
            // A folder folded into another's box is that box; in several
            // regions, the one holding the most of it.
            return self
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| matches!(node.kind, Kind::Folder | Kind::Component))
                .filter(|(_, node)| {
                    let at = node.place.trim_end_matches('/');
                    at == dir || at.starts_with(&format!("{dir}/")) || dir.is_empty()
                })
                .min_by_key(|(_, node)| (node.place.len(), std::cmp::Reverse(node.functions)))
                .map(|(ix, _)| ix)
                .or_else(|| self.box_holding_folder(dir));
        }
        self.index_of(&format!("file:{path}"))
    }

    /// The box standing for folder `dir` when it was folded away: its one
    /// file's, else the folder holding it.
    fn box_holding_folder(&self, dir: &str) -> Option<usize> {
        let inside = |place: &str| {
            place
                .split('#')
                .next()
                .and_then(|file| file.rsplit_once('/'))
                .is_some_and(|(at, _)| at == dir)
        };
        let files: Vec<usize> = (0..self.nodes.len())
            .filter(|&ix| matches!(self.nodes[ix].kind, Kind::File { .. }))
            .filter(|&ix| inside(&self.nodes[ix].place))
            .collect();
        if let [one] = files.as_slice() {
            return Some(*one);
        }
        self.nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| matches!(node.kind, Kind::Folder | Kind::Component))
            .filter(|(_, node)| dir.starts_with(node.place.as_str()))
            .max_by_key(|(_, node)| node.place.len())
            .map(|(ix, _)| ix)
    }

    /// Whether node `ancestor` holds node `node`, or is it.
    #[cfg(any(test, feature = "testing"))]
    pub fn holds(&self, ancestor: usize, node: usize) -> bool {
        std::iter::successors(Some(node), |&n| self.nodes[n].parent).any(|n| n == ancestor)
    }

    /// Node `ix` and the boxes it's in, innermost first.
    pub fn ancestry(&self, ix: usize) -> impl Iterator<Item = usize> + '_ {
        std::iter::successors(Some(ix), |&n| self.nodes[n].parent)
    }

    /// For each node, the box standing for it with `open` saying which
    /// boxes are open: itself if every box it's in is open, else the
    /// outermost closed one it's in.
    pub fn shown_as(&self, open: impl Fn(usize) -> bool) -> Vec<usize> {
        let mut shown = vec![0; self.nodes.len()];
        for ix in 1..self.nodes.len() {
            let parent = self.nodes[ix].parent.unwrap_or(0);
            shown[ix] = if shown[parent] == parent && open(parent) {
                ix
            } else {
                shown[parent]
            };
        }
        shown
    }
}

/// Whether a file is a test, by where it is and what it's called.
pub fn is_test(path: &Path) -> bool {
    let folders = path.parent().into_iter().flat_map(|dir| dir.components());
    let in_tests = folders
        .map(|c| c.as_os_str().to_string_lossy().to_lowercase())
        .any(|name| {
            matches!(
                name.as_str(),
                "test" | "tests" | "__tests__" | "spec" | "specs"
            )
        });
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem = name.split('.').next().unwrap_or("");
    in_tests
        || name.contains(".test.")
        || name.contains(".spec.")
        || stem.ends_with("Test")
        || stem.ends_with("Tests")
        || stem.ends_with("_test")
        || stem.starts_with("test_")
}

#[cfg(test)]
mod tests;
