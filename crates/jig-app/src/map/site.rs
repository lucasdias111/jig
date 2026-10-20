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
//! side; calls leaving a region end at its edge, naming the domain they
//! go to.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use super::domains::{Domains, Home, all_files, place_of};
use super::index::{Project, Target};
use super::layout::Rect;
use super::plan::slashed;

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
    /// Calls into and out of Shared.
    pub shared: bool,
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

/// Calls leaving an open region for another, or coming in from one,
/// ending at its edge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stub {
    pub region: usize,
    pub other: usize,
    pub outgoing: bool,
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
    /// The project at `project`, shared out among `domains`, laid out.
    pub fn of(project: &Project, domains: &Domains, show: Show) -> Site {
        let mut site = Site {
            show,
            ..Site::default()
        };
        let name = project
            .root
            .file_name()
            .map_or("Project".into(), |name| name.to_string_lossy().into_owned());
        site.push(Node {
            key: "dir:".into(),
            kind: Kind::Root,
            title: name.clone(),
            purpose: project.readmes.get(Path::new("")).cloned(),
            detail: String::new(),
            contents: String::new(),
            place: String::new(),
            parent: None,
            children: Vec::new(),
            rect: Rect::default(),
            header: 0.,
            functions: 0,
            open_at: 0.,
            region: 0,
        });
        let (units, homes) = units(project, domains, show);
        // The regions: every domain, and Shared and Unsorted when anything
        // lives there, tests or not.
        let mut regions: Vec<(Home, String, String)> = domains
            .domains
            .iter()
            .enumerate()
            .map(|(ix, domain)| {
                (
                    Home::Domain(ix),
                    format!("domain:{}", domain.name),
                    domain.name.clone(),
                )
            })
            .collect();
        if homes.contains(&Home::Shared) {
            regions.push((Home::Shared, "domain:/shared".into(), "Shared".into()));
        }
        if homes.contains(&Home::Unsorted) {
            regions.push((Home::Unsorted, "domain:/unsorted".into(), "Unsorted".into()));
        }
        let mut leaves: HashMap<(usize, Option<String>), usize> = HashMap::new();
        for (home, key, title) in regions {
            let purpose = match home {
                Home::Domain(ix) => Some(domains.domains[ix].purpose.clone()),
                Home::Shared => Some("Code used everywhere.".into()),
                Home::Unsorted => {
                    Some("New files no domain holds yet. Update Domains places them.".into())
                }
            }
            .filter(|purpose| !purpose.is_empty());
            let region = site.push(Node {
                key,
                kind: Kind::Region(home),
                title,
                purpose,
                detail: String::new(),
                contents: String::new(),
                place: String::new(),
                parent: Some(0),
                children: Vec::new(),
                rect: Rect::default(),
                header: REGION_HEADER,
                functions: 0,
                open_at: 0.,
                region: 0,
            });
            site.nodes[region].region = region;
            let mine: Vec<usize> = (0..units.len())
                .filter(|&u| units[u].home == home)
                .collect();
            site.add_components(project, &units, &mine, region, &name, &mut leaves);
        }
        site.count(0);
        site.size(0);
        site.place_regions();
        site.position(0, 0., 0.);
        site.open_points();
        site.calls = calls(project, &leaves);
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
    #[cfg(test)]
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

    /// The arrows between the boxes showing, `shown` from `shown_as`: each
    /// call drawn between the two boxes side by side that hold its ends,
    /// summed, one arrow per direction; and the calls between regions as
    /// stubs at the edge of an open one. Calls into and out of Shared only
    /// when asked for. Most calls first.
    pub fn arrows(&self, shown: &[usize]) -> (Vec<Arrow>, Vec<Stub>) {
        let mut summed: HashMap<(usize, usize), Calls> = HashMap::new();
        let mut stubs: HashMap<(usize, usize, bool), Calls> = HashMap::new();
        for (&(from, to), calls) in &self.calls {
            let (ra, rb) = (self.nodes[from].region, self.nodes[to].region);
            if !self.show.shared
                && (self.nodes[ra].kind == Kind::Region(Home::Shared)
                    || self.nodes[rb].kind == Kind::Region(Home::Shared))
            {
                continue;
            }
            let (a, b) = (shown[from], shown[to]);
            if a == b {
                continue;
            }
            if ra != rb {
                if shown[ra] == ra && a != ra {
                    stubs.entry((ra, rb, true)).or_default().add(calls);
                }
                if shown[rb] == rb && b != rb {
                    stubs.entry((rb, ra, false)).or_default().add(calls);
                }
                continue;
            }
            let Some((a, b)) = self.side_by_side(a, b) else {
                continue;
            };
            summed.entry((a, b)).or_default().add(calls);
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
        let mut stubs: Vec<Stub> = stubs
            .into_iter()
            .map(|((region, other, outgoing), calls)| Stub {
                region,
                other,
                outgoing,
                calls,
            })
            .collect();
        stubs.sort_by(|a, b| {
            b.calls
                .count
                .cmp(&a.calls.count)
                .then((a.region, a.other, a.outgoing).cmp(&(b.region, b.other, b.outgoing)))
        });
        (arrows, stubs)
    }

    /// The two boxes side by side, in the box holding both, that hold `a`
    /// and `b`; none when one holds the other.
    fn side_by_side(&self, a: usize, b: usize) -> Option<(usize, usize)> {
        let up: Vec<usize> = self.ancestry(a).collect();
        let mut below_b = b;
        for n in self.ancestry(b) {
            if let Some(at) = up.iter().position(|&m| m == n) {
                if at == 0 || n == b {
                    return None;
                }
                return Some((up[at - 1], below_b));
            }
            below_b = n;
        }
        None
    }

    /// Region `region`'s components, each with what's in it.
    fn add_components(
        &mut self,
        project: &Project,
        units: &[Unit],
        mine: &[usize],
        region: usize,
        project_name: &str,
        leaves: &mut HashMap<(usize, Option<String>), usize>,
    ) {
        let mut components: BTreeMap<PathBuf, Dir> = BTreeMap::new();
        for &u in mine {
            let path = &units[u].path;
            let component = component_of(project, path);
            let rest = path
                .parent()
                .unwrap_or(Path::new(""))
                .strip_prefix(&component)
                .unwrap_or(Path::new(""));
            let folders: Vec<String> = rest
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            components.entry(component).or_default().insert(&folders, u);
        }
        for (dir, tree) in &components {
            let mut title = if dir.as_os_str().is_empty() {
                project_name.to_string()
            } else {
                file_name(dir)
            };
            let (mut entries, mut own) = shown(tree, dir);
            // A component holding one folder is that folder's box.
            while own.is_empty() && entries.len() == 1 {
                let (name, path, inner) = entries.remove(0);
                if !BOILERPLATE.contains(&name.as_str()) {
                    title = format!("{title} › {name}");
                }
                (entries, own) = shown(inner, &path);
            }
            let deepest = deepest_place(dir, tree);
            let purpose = project
                .readmes
                .get(dir)
                .cloned()
                .or_else(|| project.readmes.get(Path::new(&deepest)).cloned());
            let ix = self.push(Node {
                key: format!("dir:{region}:{}/", slashed(dir)),
                kind: Kind::Component,
                title,
                purpose,
                detail: String::new(),
                contents: String::new(),
                place: format!("{}/", deepest.trim_end_matches('/')),
                parent: Some(region),
                children: Vec::new(),
                rect: Rect::default(),
                header: TITLE,
                functions: 0,
                open_at: 0.,
                region,
            });
            self.add_entries(project, units, entries, own, ix, region, leaves);
        }
    }

    /// Folders `entries` and units `own` under node `parent`.
    #[allow(clippy::too_many_arguments)]
    fn add_entries(
        &mut self,
        project: &Project,
        units: &[Unit],
        entries: Vec<(String, PathBuf, &Dir)>,
        own: &[usize],
        parent: usize,
        region: usize,
        leaves: &mut HashMap<(usize, Option<String>), usize>,
    ) {
        for (name, path, dir) in entries {
            // A folder holding just one file is the file's box.
            if dir.dirs.is_empty() && dir.units.len() == 1 {
                self.add_unit(
                    project,
                    &units[dir.units[0]],
                    Some(&name),
                    parent,
                    region,
                    leaves,
                );
                continue;
            }
            let (inner, inner_own) = shown(dir, &path);
            let place = format!("{}/", slashed(&path));
            let purpose = project
                .readmes
                .get(&path)
                .cloned()
                .or_else(|| folder_doc(project, &path));
            let ix = self.push(Node {
                key: format!("dir:{region}:{place}"),
                kind: Kind::Folder,
                title: name,
                purpose,
                detail: String::new(),
                contents: String::new(),
                place,
                parent: Some(parent),
                children: Vec::new(),
                rect: Rect::default(),
                header: TITLE,
                functions: 0,
                open_at: 0.,
                region,
            });
            self.add_entries(project, units, inner, inner_own, ix, region, leaves);
        }
        for &u in own {
            self.add_unit(project, &units[u], None, parent, region, leaves);
        }
    }

    /// A file or type as a box, titled after the folder it stands for too
    /// when it's that folder's only file; a file's types inside it when
    /// it has several.
    fn add_unit(
        &mut self,
        project: &Project,
        unit: &Unit,
        folder: Option<&str>,
        parent: usize,
        region: usize,
        leaves: &mut HashMap<(usize, Option<String>), usize>,
    ) {
        let _ = project;
        let name = match &unit.own_type {
            Some(ty) => ty.clone(),
            None => file_name(&unit.path),
        };
        let title = match folder {
            Some(folder) => format!("{folder} › {name}"),
            None => name,
        };
        let (key, place, kind) = match &unit.own_type {
            Some(ty) => (
                format!("type:{}#{ty}", unit.place),
                format!("{}#{ty}", unit.place),
                Kind::Type,
            ),
            None => (
                format!("file:{}", unit.place),
                unit.place.clone(),
                Kind::File {
                    read: unit.file.is_some(),
                },
            ),
        };
        let ix = self.push(Node {
            key,
            kind,
            title,
            purpose: unit.purpose.clone(),
            detail: String::new(),
            contents: String::new(),
            place,
            parent: Some(parent),
            children: Vec::new(),
            rect: Rect::default(),
            header: TITLE,
            functions: unit.functions,
            open_at: f32::INFINITY,
            region,
        });
        let Some(file) = unit.file else {
            return;
        };
        leaves.insert((file, unit.own_type.clone()), ix);
        // Its types inside it when it has several with code in them.
        let types: Vec<_> = unit.types.iter().filter(|ty| ty.3 > 0).collect();
        if unit.own_type.is_some() || types.len() < 2 {
            return;
        }
        for (ty, kind, purpose, functions) in types {
            let at = self.push(Node {
                key: format!("type:{}#{ty}", unit.place),
                kind: Kind::Type,
                title: format!("{kind} {ty}"),
                purpose: purpose.clone(),
                detail: String::new(),
                contents: String::new(),
                place: format!("{}#{ty}", unit.place),
                parent: Some(ix),
                children: Vec::new(),
                rect: Rect::default(),
                header: TITLE,
                functions: *functions,
                open_at: f32::INFINITY,
                region,
            });
            leaves.insert((file, Some(ty.clone())), at);
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

    /// Each box's count of what it holds, its detail line and what's in
    /// it by name.
    fn count(&mut self, ix: usize) -> (usize, usize) {
        let children = self.nodes[ix].children.clone();
        let node_kind = self.nodes[ix].kind;
        let (files, functions) = match node_kind {
            Kind::Type => (0, self.nodes[ix].functions),
            Kind::File { .. } => {
                for &child in &children {
                    self.count(child);
                }
                (1, self.nodes[ix].functions)
            }
            _ => {
                let (mut files, mut functions) = (0, 0);
                for &child in &children {
                    let (f, n) = self.count(child);
                    files += f;
                    functions += n;
                }
                self.nodes[ix].functions = functions;
                (files, functions)
            }
        };
        self.nodes[ix].detail = match node_kind {
            Kind::Type => plural(functions, "function"),
            Kind::File { read: false } => "not read".into(),
            Kind::File { .. } => plural(functions, "function"),
            _ if functions > 0 => format!(
                "{} · {}",
                plural(files, "file"),
                plural(functions, "function")
            ),
            _ => plural(files, "file"),
        };
        let names: Vec<String> = children
            .iter()
            .map(|&c| self.nodes[c].title.clone())
            .collect();
        self.nodes[ix].contents = summary(&names);
        (files, functions)
    }

    /// Node `ix`'s size, from what's in it, its children placed relative
    /// to it.
    fn size(&mut self, ix: usize) {
        let children = self.nodes[ix].children.clone();
        for &child in &children {
            self.size(child);
        }
        let kind = self.nodes[ix].kind;
        let (width, height) = match kind {
            Kind::Root => (0., 0.),
            Kind::Type => (fitted(&self.nodes[ix].title, TYPE_WIDTH), TYPE_HEIGHT),
            Kind::File { .. } if children.is_empty() => {
                (fitted(&self.nodes[ix].title, FILE_WIDTH), FILE_HEIGHT)
            }
            _ => {
                let region = matches!(kind, Kind::Region(_));
                let pad = if region { REGION_PAD } else { PAD };
                let sizes: Vec<(f32, f32)> = children
                    .iter()
                    .map(|&c| (self.nodes[c].rect.width, self.nodes[c].rect.height))
                    .collect();
                let (spots, inner_width, inner_height) = shelves(&sizes, GAP, ASPECT);
                let minimum = if region {
                    REGION_MIN_WIDTH
                } else {
                    fitted(&self.nodes[ix].title, FILE_WIDTH) + 2. * pad
                };
                let width = (inner_width + 2. * pad).max(minimum);
                let header = if region {
                    REGION_HEADER.max(width * 0.11)
                } else {
                    TITLE.max(width * 0.045)
                };
                self.nodes[ix].header = header;
                for (&child, (x, y)) in children.iter().zip(spots) {
                    self.nodes[child].rect.x = pad + x;
                    self.nodes[child].rect.y = header + y;
                }
                let height = header + inner_height + pad;
                let height = if region {
                    height.max(width / ASPECT * 0.55)
                } else {
                    height
                };
                (width, height)
            }
        };
        self.nodes[ix].rect.width = width;
        self.nodes[ix].rect.height = height;
    }

    /// The domains in rows, as near `ASPECT` as they pack, Shared and
    /// Unsorted apart below them; the project's box round them all.
    fn place_regions(&mut self) {
        let regions: Vec<usize> = self.regions().collect();
        let (domains, apart): (Vec<usize>, Vec<usize>) = regions
            .into_iter()
            .partition(|&ix| matches!(self.nodes[ix].kind, Kind::Region(Home::Domain(_))));
        let sizes: Vec<(f32, f32)> = domains
            .iter()
            .map(|&ix| (self.nodes[ix].rect.width, self.nodes[ix].rect.height))
            .collect();
        let (spots, _, height) = shelves(&sizes, REGION_GAP, REGIONS_ASPECT);
        for (&ix, (x, y)) in domains.iter().zip(spots) {
            self.nodes[ix].rect.x = x;
            self.nodes[ix].rect.y = y;
        }
        let below = if domains.is_empty() {
            0.
        } else {
            height + APART
        };
        let mut x = 0.;
        for ix in apart {
            self.nodes[ix].rect.x = x;
            self.nodes[ix].rect.y = below;
            x += self.nodes[ix].rect.width + REGION_GAP;
        }
        let bounds = Rect::around(self.regions().map(|ix| self.nodes[ix].rect)).unwrap_or_default();
        self.nodes[0].rect = bounds;
        // Children are placed relative to the project's corner.
        for ix in self.regions().collect::<Vec<_>>() {
            self.nodes[ix].rect.x -= bounds.x;
            self.nodes[ix].rect.y -= bounds.y;
        }
        self.nodes[0].rect.x = 0.;
        self.nodes[0].rect.y = 0.;
        let _ = bounds;
    }

    /// Node `ix` moved by `(x, y)` from where its parent put it, and
    /// what's in it along with it.
    fn position(&mut self, ix: usize, x: f32, y: f32) {
        if ix != 0 {
            self.nodes[ix].rect.x += x;
            self.nodes[ix].rect.y += y;
        }
        let (x, y) = (self.nodes[ix].rect.x, self.nodes[ix].rect.y);
        for child in self.nodes[ix].children.clone() {
            self.position(child, x, y);
        }
    }

    /// The zoom each box starts opening at: when it's wide enough on
    /// screen, and a typical box inside it would read.
    fn open_points(&mut self) {
        for ix in 1..self.nodes.len() {
            let node = &self.nodes[ix];
            if node.children.is_empty() {
                continue;
            }
            let mut widths: Vec<f32> = node
                .children
                .iter()
                .map(|&c| self.nodes[c].rect.width)
                .collect();
            widths.sort_by(f32::total_cmp);
            let typical = widths[widths.len() / 2].max(1.);
            let from = if matches!(node.kind, Kind::Region(_)) {
                REGION_OPEN
            } else {
                OPEN_FROM
            };
            let width = node.rect.width.max(1.);
            self.nodes[ix].open_at = (from / width).max((READ / typical).min(OPEN_BY / width));
        }
    }
}

/// The files and types each region holds, and which homes are used.
fn units(project: &Project, domains: &Domains, show: Show) -> (Vec<Unit>, Vec<Home>) {
    let read: HashMap<&Path, usize> = project
        .files
        .iter()
        .enumerate()
        .map(|(ix, file)| (file.path.as_path(), ix))
        .collect();
    let mut functions: HashMap<(usize, Option<&str>), usize> = HashMap::new();
    for function in &project.functions {
        *functions.entry((function.file, None)).or_default() += 1;
        if let Some(owner) = &function.outline.owner {
            *functions
                .entry((function.file, Some(owner.as_str())))
                .or_default() += 1;
        }
    }
    let mut units = Vec::new();
    let mut homes = Vec::new();
    for path in all_files(project) {
        let place = place_of(&path);
        let home = domains.home_of(&place);
        if !homes.contains(&home) {
            homes.push(home);
        }
        if !show.tests && is_test(&path) {
            continue;
        }
        let file = read.get(path.as_path()).copied();
        let mut count = file
            .and_then(|ix| functions.get(&(ix, None)))
            .copied()
            .unwrap_or(0);
        let mut types = Vec::new();
        if let Some(ix) = file {
            for ty in &project.files[ix].types {
                let n = functions
                    .get(&(ix, Some(ty.name.as_str())))
                    .copied()
                    .unwrap_or(0);
                let own = domains.home_of(&format!("{place}#{}", ty.name));
                if own == home {
                    types.push((ty.name.clone(), ty.kind, ty.purpose.clone(), n));
                    continue;
                }
                if !homes.contains(&own) {
                    homes.push(own);
                }
                count = count.saturating_sub(n);
                units.push(Unit {
                    path: path.clone(),
                    place: place.clone(),
                    home: own,
                    file,
                    own_type: Some(ty.name.clone()),
                    purpose: ty.purpose.clone(),
                    functions: n,
                    types: Vec::new(),
                });
            }
        }
        // A file holding one type is that type's box.
        let purpose = file.and_then(|ix| {
            project.files[ix]
                .purpose
                .clone()
                .or_else(|| match types.as_slice() {
                    [(_, _, purpose, _)] => purpose.clone(),
                    _ => None,
                })
        });
        units.push(Unit {
            path,
            place,
            home,
            file,
            own_type: None,
            purpose,
            functions: count,
            types,
        });
    }
    (units, homes)
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

/// The component holding `path`: the deepest package (a folder with a
/// manifest) it's in, else its top folder, else the project.
fn component_of(project: &Project, path: &Path) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new(""));
    project
        .packages
        .iter()
        .filter(|package| !package.as_os_str().is_empty() && dir.starts_with(package))
        .max_by_key(|package| package.components().count())
        .cloned()
        .unwrap_or_else(|| {
            dir.components()
                .next()
                .map(|c| PathBuf::from(c.as_os_str()))
                .unwrap_or_default()
        })
}

/// What folder `dir` shows: its folders, with those that only lead
/// somewhere (a chain of single folders, `src`, `main`) folded away, and
/// the units right in it.
fn shown<'a>(dir: &'a Dir, at: &Path) -> (Vec<(String, PathBuf, &'a Dir)>, &'a [usize]) {
    fn lift<'a>(
        name: &str,
        path: PathBuf,
        dir: &'a Dir,
        out: &mut Vec<(String, PathBuf, &'a Dir)>,
    ) {
        if dir.units.is_empty() && (dir.dirs.len() == 1 || BOILERPLATE.contains(&name)) {
            for (inner, sub) in &dir.dirs {
                lift(inner, path.join(inner), sub, out);
            }
        } else {
            out.push((name.to_string(), path, dir));
        }
    }
    let mut out = Vec::new();
    for (name, sub) in &dir.dirs {
        lift(name, at.join(name), sub, &mut out);
    }
    (out, &dir.units)
}

/// The deepest folder a component leads straight down to.
fn deepest_place(dir: &Path, tree: &Dir) -> String {
    let mut path = dir.to_path_buf();
    let mut at = tree;
    while at.units.is_empty() && at.dirs.len() == 1 {
        let (name, inner) = at.dirs.iter().next().expect("one");
        path = path.join(name);
        at = inner;
    }
    slashed(&path)
}

/// Boxes of these sizes in rows, as near `aspect` as they pack: where
/// each goes, and how wide and tall they come to.
fn shelves(sizes: &[(f32, f32)], gap: f32, aspect: f32) -> (Vec<(f32, f32)>, f32, f32) {
    let area: f32 = sizes.iter().map(|(w, h)| (w + gap) * (h + gap)).sum();
    let widest = sizes.iter().map(|(w, _)| *w).fold(0., f32::max);
    let row = (area * aspect).sqrt().max(widest);
    let mut spots = Vec::with_capacity(sizes.len());
    let (mut x, mut y, mut tallest, mut width) = (0., 0., 0f32, 0f32);
    for &(w, h) in sizes {
        if x > 0. && x + w > row {
            x = 0.;
            y += tallest + gap;
            tallest = 0.;
        }
        spots.push((x, y));
        x += w + gap;
        width = width.max(x - gap);
        tallest = tallest.max(h);
    }
    (spots, width, y + tallest)
}

/// A box wide enough for its title, at the size Site draws titles.
fn fitted(title: &str, at_least: f32) -> f32 {
    (title.chars().count() as f32 * 7.4 + 26.).clamp(at_least, FILE_MAX_WIDTH)
}

/// Names in a line: the first few, and how many more.
fn summary(names: &[String]) -> String {
    const SHOWN: usize = 4;
    let mut out = names
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ");
    if names.len() > SHOWN {
        out.push_str(&format!(" · +{}", names.len() - SHOWN));
    }
    out
}

/// What a folder's own module file says it's for: `mod.rs`, the
/// `name.rs` beside it, `__init__.py`, `index.ts`, `doc.go`,
/// `package-info.java`.
fn folder_doc(project: &Project, dir: &Path) -> Option<String> {
    let name = dir.file_name()?.to_string_lossy().into_owned();
    let candidates = [
        dir.join("mod.rs"),
        dir.with_file_name(format!("{name}.rs")),
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
            .find(|file| &file.path == path)?
            .purpose
            .clone()
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
    leaves: &HashMap<(usize, Option<String>), usize>,
) -> HashMap<(usize, usize), Calls> {
    let unit = |id: usize| {
        let function = &project.functions[id];
        leaves
            .get(&(function.file, function.outline.owner.clone()))
            .or_else(|| leaves.get(&(function.file, None)))
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

    fn domains() -> Domains {
        toml::from_str(
            r#"
[[domain]]
name = "Bug Reports"
purpose = "Report what's broken."
members = ["server/src/org/demo/bugs/", "client/src/rooms.ts"]

[[domain]]
name = "People"
purpose = "Who uses it."
members = ["server/src/org/demo/users/"]

[shared]
members = ["client/src/net/"]
"#,
        )
        .unwrap()
    }

    #[test]
    fn each_region_holds_its_share_of_each_component() {
        let dir = fixture();
        std::fs::write(
            dir.path().join("client/src/App.svelte"),
            "<script></script>",
        )
        .unwrap();
        let project = Project::read(dir.path());
        let site = Site::of(&project, &domains(), Show::default());
        assert_eq!(
            titles(&site, 0),
            ["Bug Reports", "People", "Shared", "Unsorted"]
        );
        let bugs = site.index_of("domain:Bug Reports").unwrap();
        // Its slice of both components, the server's chain folded away.
        assert_eq!(titles(&site, bugs), ["client", "server › bugs"]);
        let server = site.nodes[bugs].children[1];
        assert_eq!(
            titles(&site, server),
            ["BugService.java", "MailNotifier.java", "Notifier.java"]
        );
        // A file the index can't read is in the region its folder is in.
        let unsorted = site.index_of("domain:/unsorted").unwrap();
        assert!(site.holds(unsorted, site.box_of("client/src/App.svelte").unwrap()));
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
    fn tests_show_only_when_asked() {
        let dir = fixture();
        let test = dir
            .path()
            .join("server/src/test/org/demo/bugs/BugServiceTest.java");
        std::fs::create_dir_all(test.parent().unwrap()).unwrap();
        std::fs::write(&test, "class BugServiceTest { void makes() {} }\n").unwrap();
        let project = Project::read(dir.path());
        let place = "file:server/src/test/org/demo/bugs/BugServiceTest.java";
        let site = Site::of(&project, &domains(), Show::default());
        assert!(site.index_of(place).is_none());
        let tests = Show {
            tests: true,
            ..Show::default()
        };
        assert!(
            Site::of(&project, &domains(), tests)
                .index_of(place)
                .is_some()
        );
    }

    #[test]
    fn arrows_join_boxes_side_by_side_and_calls_leaving_end_at_the_edge() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let site = Site::of(&project, &domains(), Show::default());
        let bugs = site.index_of("domain:Bug Reports").unwrap();
        let people = site.index_of("domain:People").unwrap();
        let service = site
            .box_of("server/src/org/demo/bugs/BugService.java")
            .unwrap();
        let mail = site
            .box_of("server/src/org/demo/bugs/MailNotifier.java")
            .unwrap();
        // Everything open: BugService calls into People, out at the edge.
        let open = site.shown_as(|_| true);
        let (arrows, stubs) = site.arrows(&open);
        let out = stubs
            .iter()
            .find(|s| s.region == bugs && s.other == people && s.outgoing)
            .expect("a stub to People");
        assert_eq!(out.calls.examples, ["BugService.create → UserService.find"]);
        assert!(
            stubs
                .iter()
                .any(|s| s.region == people && s.other == bugs && !s.outgoing)
        );
        // Calls inside a region join the boxes side by side.
        assert!(
            arrows
                .iter()
                .all(|a| site.nodes[a.from].parent == site.nodes[a.to].parent)
        );
        let _ = (service, mail);
        // Regions closed: no stubs, those are the domains' flows.
        let closed = site.shown_as(|ix| ix == 0);
        let (arrows, stubs) = site.arrows(&closed);
        assert!(arrows.is_empty() && stubs.is_empty());
    }

    #[test]
    fn calls_inside_a_folder_join_its_boxes_not_the_deep_ones() {
        let dir = fixture();
        let project = Project::read(dir.path());
        // Everything in one region.
        let site = Site::of(&project, &Domains::default(), Show::default());
        let server = site.box_of("server/src/").unwrap();
        let client = site.box_of("client/src/").unwrap();
        let service = site
            .box_of("server/src/org/demo/bugs/BugService.java")
            .unwrap();
        let open = site.shown_as(|_| true);
        let (arrows, _) = site.arrows(&open);
        // BugService → UserService: both inside `server`, between its
        // folders `bugs` and `users`.
        let bugs = site.nodes[service].parent.unwrap();
        let users = site.box_of("server/src/org/demo/users/").unwrap();
        assert!(arrows.iter().any(|a| a.from == bugs && a.to == users));
        assert!(
            arrows
                .iter()
                .filter(|a| a.from == service)
                .all(|a| site.nodes[a.to].parent == Some(bugs))
        );
        assert!(!arrows.iter().any(|a| a.from == server || a.from == client));
    }

    #[test]
    fn places_land_on_the_box_holding_them() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let site = Site::of(&project, &Domains::default(), Show::default());
        let file = site.box_of("client/src/rooms.ts").unwrap();
        assert_eq!(site.nodes[file].title, "rooms.ts");
        let users = site.box_of("server/src/org/demo/users/").unwrap();
        assert!(site.nodes[users].title.contains("UserService.java"));
        // A Java file holds one type: one box, the type's.
        assert!(
            site.box_of("server/src/org/demo/bugs/Notifier.java#Notifier")
                .is_some_and(|ix| site.nodes[ix].kind == Kind::File { read: true })
        );
    }

    #[test]
    fn boxes_open_once_their_insides_read() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let site = Site::of(&project, &Domains::default(), Show::default());
        let region = site.regions().next().unwrap();
        let node = &site.nodes[region];
        assert_eq!(node.opening(node.open_at * 0.99), 0.);
        assert_eq!(node.opening(node.open_to() * 1.01), 1.);
        for &child in &node.children {
            let typical = site.nodes[child].rect.width * node.open_at;
            assert!(typical >= READ * 0.5, "{typical}");
        }
    }

    /// The Map on a big project stays smooth: `JIG_MAP_PROJECT=<a repo of
    /// about 10k files> cargo test -p jig-app big_project_live -- --ignored
    /// --nocapture` (budgets are for a release build; a debug build gets
    /// eight times as long). Reading it, reading it again unchanged,
    /// laying Site out, what each frame at Site works out, and a Plan.
    #[test]
    #[ignore]
    fn big_project_live() {
        use std::time::{Duration, Instant};
        let root = std::env::var_os("JIG_MAP_PROJECT").expect("JIG_MAP_PROJECT");
        let root = Path::new(&root);
        let slack = if cfg!(debug_assertions) { 8 } else { 1 };
        let within = |what: &str, took: Duration, budget_ms: u64| {
            println!("{what}: {took:?}");
            assert!(
                took <= Duration::from_millis(budget_ms * slack),
                "{what} took {took:?}, over {budget_ms} ms"
            );
        };
        let started = Instant::now();
        let project = Project::read(root);
        within("read", started.elapsed(), 20_000);
        println!(
            "{} files read, {} placed by folder, {} functions, {} skipped",
            project.files.len(),
            project.others.len(),
            project.functions.len(),
            project.skipped
        );
        let started = Instant::now();
        let again = Project::read_again(root, Some(&project));
        within("read again, unchanged", started.elapsed(), 4_000);
        assert_eq!(again.functions.len(), project.functions.len());
        let started = Instant::now();
        let grouping = super::super::domains::grouping::Grouping::of(&project);
        let site = Site::of(&project, &grouping.domains(), Show::default());
        within("grouped, and Site laid out", started.elapsed(), 4_000);
        println!(
            "{} boxes, {} counted calls",
            site.nodes.len(),
            site.calls.len()
        );
        // Each frame at Site: which boxes show, and the arrows between them,
        // all the way out and further in.
        let fit = 1000. / site.nodes[0].rect.width.max(1.);
        for zoom in [fit, fit * 4., fit * 16., fit * 64.] {
            let started = Instant::now();
            let shown = site.shown_as(|ix| site.nodes[ix].opening(zoom) >= 0.5);
            let (arrows, _) = site.arrows(&shown);
            within(
                &format!("a frame at {zoom:.3}x ({} arrows)", arrows.len()),
                started.elapsed(),
                30,
            );
        }
        // A Plan around the most called function.
        let busiest = (0..project.functions.len())
            .max_by_key(|&id| project.callers[id].len())
            .expect("a function");
        let started = Instant::now();
        let plan = super::super::plan::neighbourhood(
            &project,
            busiest,
            &super::super::plan::Opened::default(),
        );
        super::super::layout::layout(&plan);
        within(
            &format!("the Plan of {}", project.functions[busiest].title()),
            started.elapsed(),
            30,
        );
        // Seven callers and "+N more", and its tests folded into one.
        let callers = plan.nodes.iter().filter(|n| n.column == -1).count();
        assert!(
            callers <= super::super::plan::MAX_SIDE + 1,
            "{callers} callers"
        );
    }
}
