//! The Site level: the whole project as one nested drawing. Folders are
//! frames holding their folders and files, a file read by the outline
//! holds its types, and folders a manifest makes a package are the outer
//! frames. Everything is laid out once, every folder at the size its
//! contents take, so opening one as you zoom in only shows what's inside
//! its box: nothing moves, and you never change screens.
//!
//! Each box has a one-line purpose where the code gives one (a README, a
//! file's doc comment). Calls between files and types are counted, so
//! that whichever boxes are showing, one arrow per direction says how
//! many calls go between them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::index::{Project, Target};
use super::layout::Rect;
use super::plan::slashed;

/// A file box, closed or without types.
const FILE_WIDTH: f32 = 200.;
const FILE_HEIGHT: f32 = 64.;
const TYPE_WIDTH: f32 = 170.;
const TYPE_HEIGHT: f32 = 48.;
/// A frame's title bar, and the room round what's in it.
pub const TITLE: f32 = 34.;
const PAD: f32 = 14.;
const GAP: f32 = 14.;
/// How much wider than tall a folder's contents are packed.
const ASPECT: f32 = 1.6;
/// Examples kept of the calls behind an arrow, for its tooltip.
const MAX_EXAMPLES: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The project itself.
    Root,
    /// A folder; a package or service when a manifest says so.
    Folder { package: bool },
    /// A file, read by the outline or only placed by its folder.
    File { read: bool },
    /// A type declared in a file.
    Type,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// What keeps it in saved places and selections: `dir:a/b/`,
    /// `file:a/b.rs`, `type:a/b.rs#B`.
    pub key: String,
    pub kind: Kind,
    pub title: String,
    pub purpose: Option<String>,
    /// How much it holds: `12 files · 340 functions`.
    pub detail: String,
    /// As the Map names places: `a/b/`, `a/b.rs`, `a/b.rs#B`.
    pub place: String,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    /// Where it is, in unzoomed points; a folder's box is its frame.
    pub rect: Rect,
    /// How many functions it holds, all the way down.
    pub functions: usize,
}

impl Node {
    /// Whether it opens to show what's inside.
    pub fn opens(&self) -> bool {
        !self.children.is_empty()
    }
}

/// The calls from one box to another, counted.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Calls {
    pub count: usize,
    /// Some of them, as `caller → callee`.
    pub examples: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Site {
    /// In order, each before what's in it; the project's first.
    pub nodes: Vec<Node>,
    /// The calls between files and types, by (from, to) node.
    pub calls: HashMap<(usize, usize), Calls>,
    by_key: HashMap<String, usize>,
}

/// An arrow between two boxes showing: one per direction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arrow {
    pub from: usize,
    pub to: usize,
    pub calls: Calls,
}

impl Site {
    /// The project at `project`, laid out.
    pub fn of(project: &Project) -> Site {
        let mut site = Site::default();
        let name = project
            .root
            .file_name()
            .map_or("Project".into(), |name| name.to_string_lossy().into_owned());
        site.nodes.push(Node {
            key: "dir:".into(),
            kind: Kind::Root,
            title: name,
            purpose: project.readmes.get(Path::new("")).cloned(),
            detail: String::new(),
            place: String::new(),
            parent: None,
            children: Vec::new(),
            rect: Rect::default(),
            functions: 0,
        });
        let tree = Tree::of(project);
        let mut units = HashMap::new();
        site.add_folder(project, &tree, Path::new(""), 0, &mut units);
        site.count(0);
        site.place(0);
        site.calls = calls(project, &units);
        site.by_key = site
            .nodes
            .iter()
            .enumerate()
            .map(|(ix, node)| (node.key.clone(), ix))
            .collect();
        site
    }

    pub fn index_of(&self, key: &str) -> Option<usize> {
        self.by_key.get(key).copied()
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
            // A folder merged into its only folder's box is that box.
            return self
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, node)| matches!(node.kind, Kind::Folder { .. } | Kind::Root))
                .filter(|(_, node)| {
                    let at = node.place.trim_end_matches('/');
                    at == dir || at.starts_with(&format!("{dir}/")) || dir.is_empty()
                })
                .min_by_key(|(_, node)| node.place.len())
                .map(|(ix, _)| ix);
        }
        self.index_of(&format!("file:{path}"))
    }

    /// Whether node `ancestor` holds node `node`, or is it.
    pub fn holds(&self, ancestor: usize, node: usize) -> bool {
        std::iter::successors(Some(node), |&n| self.nodes[n].parent).any(|n| n == ancestor)
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

    /// The arrows between the boxes showing, `shown` from `shown_as`: the
    /// calls between what each stands for, summed, one arrow per
    /// direction, none into a box's own frame. Most calls first.
    pub fn arrows(&self, shown: &[usize]) -> Vec<Arrow> {
        let mut summed: HashMap<(usize, usize), Calls> = HashMap::new();
        for (&(from, to), calls) in &self.calls {
            let (from, to) = (shown[from], shown[to]);
            if from == to || self.holds(from, to) || self.holds(to, from) {
                continue;
            }
            let entry = summed.entry((from, to)).or_default();
            entry.count += calls.count;
            for example in &calls.examples {
                if entry.examples.len() < MAX_EXAMPLES && !entry.examples.contains(example) {
                    entry.examples.push(example.clone());
                }
            }
        }
        let mut arrows: Vec<Arrow> = summed
            .into_iter()
            .map(|((from, to), calls)| Arrow { from, to, calls })
            .collect();
        arrows.sort_by(|a, b| {
            b.calls
                .count
                .cmp(&a.calls.count)
                .then((a.from, a.to).cmp(&(b.from, b.to)))
        });
        arrows
    }

    /// Folder `dir`'s folders and files as boxes under node `parent`.
    fn add_folder(
        &mut self,
        project: &Project,
        tree: &Tree,
        dir: &Path,
        parent: usize,
        units: &mut HashMap<(usize, Option<String>), usize>,
    ) {
        let Some(folder) = tree.folders.get(dir) else {
            return;
        };
        for sub in &folder.folders {
            // A folder holding just one folder is shown as one box:
            // `server/src/org/demo`; not a package's or one with a README.
            let mut path = sub.clone();
            while let Some(inner) = tree.folders.get(&path)
                && inner.files.is_empty()
                && inner.others.is_empty()
                && inner.folders.len() == 1
                && !project.packages.contains(&path)
                && !project.readmes.contains_key(&path)
            {
                path = inner.folders[0].clone();
            }
            let place = format!("{}/", slashed(&path));
            let title = slashed(path.strip_prefix(dir).unwrap_or(&path));
            let purpose = project
                .readmes
                .get(&path)
                .cloned()
                .or_else(|| folder_doc(project, &path));
            let ix = self.push(Node {
                key: format!("dir:{place}"),
                kind: Kind::Folder {
                    package: project.packages.contains(&path),
                },
                title,
                purpose,
                detail: String::new(),
                place,
                parent: Some(parent),
                children: Vec::new(),
                rect: Rect::default(),
                functions: 0,
            });
            self.add_folder(project, tree, &path, ix, units);
        }
        for &file in &folder.files {
            let source = &project.files[file];
            let place = slashed(&source.path);
            let functions = project.modules[source.module]
                .functions
                .iter()
                .filter(|&&id| project.functions[id].file == file)
                .count();
            let ix = self.push(Node {
                key: format!("file:{place}"),
                kind: Kind::File { read: true },
                title: file_name(&source.path),
                purpose: source.purpose.clone(),
                detail: String::new(),
                place: place.clone(),
                parent: Some(parent),
                children: Vec::new(),
                rect: Rect::default(),
                functions,
            });
            units.insert((file, None), ix);
            for ty in &source.types {
                if units.contains_key(&(file, Some(ty.name.clone()))) {
                    continue;
                }
                let owned = project.modules[source.module]
                    .functions
                    .iter()
                    .filter(|&&id| {
                        let function = &project.functions[id];
                        function.file == file
                            && function.outline.owner.as_deref() == Some(ty.name.as_str())
                    })
                    .count();
                let at = self.push(Node {
                    key: format!("type:{place}#{}", ty.name),
                    kind: Kind::Type,
                    title: format!("{} {}", ty.kind, ty.name),
                    purpose: ty.purpose.clone(),
                    detail: String::new(),
                    place: format!("{place}#{}", ty.name),
                    parent: Some(ix),
                    children: Vec::new(),
                    rect: Rect::default(),
                    functions: owned,
                });
                units.insert((file, Some(ty.name.clone())), at);
            }
        }
        for path in &folder.others {
            let place = slashed(path);
            self.push(Node {
                key: format!("file:{place}"),
                kind: Kind::File { read: false },
                title: file_name(path),
                purpose: None,
                detail: String::new(),
                place,
                parent: Some(parent),
                children: Vec::new(),
                rect: Rect::default(),
                functions: 0,
            });
        }
    }

    fn push(&mut self, node: Node) -> usize {
        let ix = self.nodes.len();
        if let Some(parent) = node.parent {
            self.nodes[parent].children.push(ix);
        }
        self.nodes.push(node);
        ix
    }

    /// Each box's count of what it holds, and its detail line.
    fn count(&mut self, ix: usize) -> (usize, usize) {
        let children = self.nodes[ix].children.clone();
        match self.nodes[ix].kind {
            Kind::Type => {
                let functions = self.nodes[ix].functions;
                self.nodes[ix].detail = plural(functions, "function");
                (0, 0)
            }
            Kind::File { read } => {
                for child in children {
                    self.count(child);
                }
                let functions = self.nodes[ix].functions;
                self.nodes[ix].detail = if read {
                    plural(functions, "function")
                } else {
                    "not read".into()
                };
                (1, functions)
            }
            Kind::Root | Kind::Folder { .. } => {
                let (mut files, mut functions) = (0, 0);
                for child in children {
                    let (f, n) = self.count(child);
                    files += f;
                    functions += n;
                }
                self.nodes[ix].functions = functions;
                self.nodes[ix].detail = if functions > 0 {
                    format!(
                        "{} · {}",
                        plural(files, "file"),
                        plural(functions, "function")
                    )
                } else {
                    plural(files, "file")
                };
                (files, functions)
            }
        }
    }

    /// Lays out node `ix` and what's in it at the origin, then moves each
    /// box into place: sizes come up from the files, places down from the
    /// project.
    fn place(&mut self, ix: usize) {
        self.size(ix);
        self.position(ix, 0., 0.);
    }

    /// Node `ix`'s size, from what's in it, its children placed
    /// relative to it.
    fn size(&mut self, ix: usize) {
        let children = self.nodes[ix].children.clone();
        for &child in &children {
            self.size(child);
        }
        let (width, height) = match self.nodes[ix].kind {
            Kind::Type => (TYPE_WIDTH, TYPE_HEIGHT),
            Kind::File { .. } if children.is_empty() => (FILE_WIDTH, FILE_HEIGHT),
            _ => {
                let sizes: Vec<(f32, f32)> = children
                    .iter()
                    .map(|&c| (self.nodes[c].rect.width, self.nodes[c].rect.height))
                    .collect();
                let (spots, width, height) = shelves(&sizes);
                for (&child, (x, y)) in children.iter().zip(spots) {
                    self.nodes[child].rect.x = PAD + x;
                    self.nodes[child].rect.y = TITLE + y;
                }
                let minimum = if matches!(self.nodes[ix].kind, Kind::File { .. }) {
                    FILE_WIDTH
                } else {
                    FILE_WIDTH + 2. * PAD
                };
                ((width + 2. * PAD).max(minimum), height + TITLE + PAD)
            }
        };
        self.nodes[ix].rect.width = width;
        self.nodes[ix].rect.height = height;
    }

    /// Node `ix` moved by `(x, y)` from where its parent put it, and
    /// what's in it along with it.
    fn position(&mut self, ix: usize, x: f32, y: f32) {
        self.nodes[ix].rect.x += x;
        self.nodes[ix].rect.y += y;
        let (x, y) = (self.nodes[ix].rect.x, self.nodes[ix].rect.y);
        for child in self.nodes[ix].children.clone() {
            self.position(child, x, y);
        }
    }
}

/// Boxes of these sizes in rows, as near `ASPECT` as they pack: where
/// each goes, and how wide and tall they come to.
fn shelves(sizes: &[(f32, f32)]) -> (Vec<(f32, f32)>, f32, f32) {
    let area: f32 = sizes.iter().map(|(w, h)| (w + GAP) * (h + GAP)).sum();
    let widest = sizes.iter().map(|(w, _)| *w).fold(0., f32::max);
    let row = (area * ASPECT).sqrt().max(widest);
    let mut spots = Vec::with_capacity(sizes.len());
    let (mut x, mut y, mut tallest, mut width) = (0., 0., 0f32, 0f32);
    for &(w, h) in sizes {
        if x > 0. && x + w > row {
            x = 0.;
            y += tallest + GAP;
            tallest = 0.;
        }
        spots.push((x, y));
        x += w + GAP;
        width = width.max(x - GAP);
        tallest = tallest.max(h);
    }
    (spots, width, y + tallest)
}

/// The project's folders, each with its folders, files read and files
/// placed.
#[derive(Default)]
struct Tree {
    folders: HashMap<PathBuf, Folder>,
}

#[derive(Default)]
struct Folder {
    folders: Vec<PathBuf>,
    files: Vec<usize>,
    others: Vec<PathBuf>,
}

impl Tree {
    fn of(project: &Project) -> Tree {
        let mut tree = Tree::default();
        tree.folders.insert(PathBuf::new(), Folder::default());
        for (ix, file) in project.files.iter().enumerate() {
            let dir = file.path.parent().unwrap_or(Path::new("")).to_path_buf();
            tree.folder(&dir).files.push(ix);
        }
        for path in &project.others {
            let dir = path.parent().unwrap_or(Path::new("")).to_path_buf();
            tree.folder(&dir).others.push(path.clone());
        }
        for folder in tree.folders.values_mut() {
            folder.folders.sort();
            folder
                .files
                .sort_by_key(|&ix| project.files[ix].path.clone());
        }
        tree
    }

    /// Folder `dir`, made with the folders it's in if it's new.
    fn folder(&mut self, dir: &Path) -> &mut Folder {
        if !self.folders.contains_key(dir) {
            self.folders.insert(dir.to_path_buf(), Folder::default());
            let parent = dir.parent().unwrap_or(Path::new("")).to_path_buf();
            if dir != Path::new("") {
                let list = &mut self.folder(&parent).folders;
                if !list.iter().any(|f| f == dir) {
                    list.push(dir.to_path_buf());
                }
            }
        }
        self.folders.get_mut(dir).expect("made above")
    }
}

/// What a folder's own module file says it's for: `mod.rs`, the
/// `name.rs` beside it, `__init__.py`, `index.ts`, `doc.go`,
/// `package-info.java`.
fn folder_doc(project: &Project, dir: &Path) -> Option<String> {
    let name = dir.file_name()?.to_string_lossy().into_owned();
    let beside = dir.with_file_name(format!("{name}.rs"));
    let candidates = [
        dir.join("mod.rs"),
        beside,
        dir.join("lib.rs"),
        dir.join("__init__.py"),
        dir.join("index.ts"),
        dir.join("index.js"),
        dir.join("doc.go"),
        dir.join("package-info.java"),
    ];
    candidates.iter().find_map(|path| {
        project
            .files
            .iter()
            .find(|file| &file.path == path)
            .and_then(|file| file.purpose.clone())
    })
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| slashed(path), |name| name.to_string_lossy().into_owned())
}

fn plural(count: usize, what: &str) -> String {
    format!("{count} {what}{}", if count == 1 { "" } else { "s" })
}

/// The calls between files and types: each certain call (or callback)
/// from a function in one to a function in another, counted.
fn calls(
    project: &Project,
    units: &HashMap<(usize, Option<String>), usize>,
) -> HashMap<(usize, usize), Calls> {
    let unit = |id: usize| {
        let function = &project.functions[id];
        units
            .get(&(function.file, function.outline.owner.clone()))
            .or_else(|| units.get(&(function.file, None)))
            .copied()
    };
    let mut calls: HashMap<(usize, usize), Calls> = HashMap::new();
    for (id, edges) in project.edges.iter().enumerate() {
        let Some(from) = unit(id) else {
            continue;
        };
        for edge in edges {
            let Target::Function(to) = edge.target else {
                continue;
            };
            let Some(into) = unit(to) else {
                continue;
            };
            if from == into {
                continue;
            }
            let entry = calls.entry((from, into)).or_default();
            entry.count += 1;
            let example = format!(
                "{} → {}",
                project.functions[id].title(),
                project.functions[to].title()
            );
            if entry.examples.len() < MAX_EXAMPLES && !entry.examples.contains(&example) {
                entry.examples.push(example);
            }
        }
    }
    calls
}

#[cfg(test)]
mod tests {
    use super::super::index::tests::fixture;
    use super::*;

    fn titles(site: &Site, ix: usize) -> Vec<&str> {
        site.nodes[ix]
            .children
            .iter()
            .map(|&c| site.nodes[c].title.as_str())
            .collect()
    }

    #[test]
    fn folders_nest_with_their_files_and_types_inside() {
        let dir = fixture();
        std::fs::write(
            dir.path().join("client/src/App.svelte"),
            "<script></script>",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("client/README.md"),
            "# Client\n\nThe web client that joins rooms. It talks to the server.\n",
        )
        .unwrap();
        let project = Project::read(dir.path());
        let site = Site::of(&project);
        assert_eq!(titles(&site, 0), ["client", "server/src/org/demo"]);
        let client = site.index_of("dir:client/").unwrap();
        assert_eq!(
            site.nodes[client].purpose.as_deref(),
            Some("The web client that joins rooms.")
        );
        let src = site.index_of("dir:client/src/").unwrap();
        // A file the index can't read is placed by its folder.
        assert_eq!(titles(&site, src), ["net", "rooms.ts", "App.svelte"]);
        let svelte = site.index_of("file:client/src/App.svelte").unwrap();
        assert_eq!(site.nodes[svelte].kind, Kind::File { read: false });
        let bugs = site
            .index_of("file:server/src/org/demo/bugs/BugService.java")
            .unwrap();
        assert_eq!(titles(&site, bugs), ["class BugService"]);
        // Everything inside the box it's in, and nothing overlapping.
        for (ix, node) in site.nodes.iter().enumerate() {
            if let Some(parent) = node.parent {
                let outer = site.nodes[parent].rect;
                assert_eq!(outer.union(&node.rect), outer, "{} sticks out", node.key);
            }
            for &other in &node.children {
                for &next in &node.children {
                    if other < next {
                        assert!(
                            !site.nodes[other].rect.overlaps(&site.nodes[next].rect),
                            "{} and {} overlap in {ix}",
                            site.nodes[other].key,
                            site.nodes[next].key
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn arrows_count_the_calls_between_the_boxes_showing() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let site = Site::of(&project);
        let users = site.box_of("server/src/org/demo/users/").unwrap();
        let bugs = site.box_of("server/src/org/demo/bugs/").unwrap();
        // Everything open: the types' calls.
        let open = site.shown_as(|_| true);
        let arrows = site.arrows(&open);
        let service = site
            .box_of("server/src/org/demo/bugs/BugService.java#BugService")
            .unwrap();
        let find = site
            .box_of("server/src/org/demo/users/UserService.java#UserService")
            .unwrap();
        let arrow = arrows
            .iter()
            .find(|a| a.from == service && a.to == find)
            .expect("BugService calls UserService");
        assert_eq!(arrow.calls.count, 1);
        assert_eq!(
            arrow.calls.examples,
            ["BugService.create → UserService.find"]
        );
        // The folders closed: their files' calls, summed onto them.
        let closed = site.shown_as(|ix| ix != bugs && ix != users);
        let arrows = site.arrows(&closed);
        assert!(arrows.iter().any(|a| a.from == bugs && a.to == users));
        assert!(!arrows.iter().any(|a| a.from == users && a.to == bugs));
        // A box holding both ends has no arrow.
        let top = site.shown_as(|ix| ix == 0);
        assert!(site.arrows(&top).iter().all(|a| a.from != a.to));
    }

    #[test]
    fn places_land_on_the_box_holding_them() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let site = Site::of(&project);
        let file = site.box_of("client/src/rooms.ts").unwrap();
        assert_eq!(site.nodes[file].title, "rooms.ts");
        // A folder merged into one box is found by any part of it.
        let demo = site.box_of("server/src/").unwrap();
        assert_eq!(site.nodes[demo].title, "server/src/org/demo");
        let ty = site
            .box_of("server/src/org/demo/bugs/Notifier.java#Notifier")
            .unwrap();
        assert_eq!(site.nodes[ty].kind, Kind::Type);
        assert!(site.holds(demo, ty));
    }
}
