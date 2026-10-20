//! The Domain level laid out: each domain a big region, sized by how much
//! code it holds, with its name, purpose and offers at the top and its
//! files and types packed below, shown as you zoom in. Shared and
//! Unsorted sit apart below the domains, smaller. Between two domains
//! there's one link, carrying what flows each way.

use std::collections::HashMap;

use super::super::index::Project;
use super::super::layout::Rect;
use super::{Domains, Home, all_files, place_of};

/// Room at a region's top for its name, purpose and offers.
pub const HEADER: f32 = 190.;
const PAD: f32 = 18.;
pub const MEMBER_WIDTH: f32 = 190.;
pub const MEMBER_HEIGHT: f32 = 46.;
const GAP: f32 = 10.;
/// Between regions: room for the links' labels.
const REGION_GAP: f32 = 200.;
/// A region's width, from the least code to the most.
const MIN_WIDTH: f32 = 560.;
const MAX_WIDTH: f32 = 1500.;
/// Between the domains and Shared and Unsorted below them.
const APART: f32 = 280.;
/// How much wider than tall regions and their members are packed.
const ASPECT: f32 = 1.6;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionKind {
    /// Domain number `ix` of the domains, also its colour.
    Domain(usize),
    Shared,
    Unsorted,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    /// `domain:<name>`, `domain:/shared` or `domain:/unsorted`.
    pub key: String,
    pub kind: RegionKind,
    pub name: String,
    pub purpose: String,
    pub offers: Vec<String>,
    pub why: String,
    pub members: Vec<Member>,
    pub rect: Rect,
    /// How much code it holds, in bytes.
    pub weight: u64,
    pub files: usize,
    pub functions: usize,
}

impl Region {
    /// How much it holds: `12 files · 340 functions`.
    pub fn detail(&self) -> String {
        let files = plural(self.files, "file");
        if self.functions > 0 {
            format!("{files} · {}", plural(self.functions, "function"))
        } else {
            files
        }
    }

    pub fn home(&self) -> Home {
        match self.kind {
            RegionKind::Domain(ix) => Home::Domain(ix),
            RegionKind::Shared => Home::Shared,
            RegionKind::Unsorted => Home::Unsorted,
        }
    }
}

/// A file, or a type with a home of its own, in its region.
#[derive(Clone, Debug, PartialEq)]
pub struct Member {
    /// `member:<place>`.
    pub key: String,
    /// `a/b.rs` or `a/b.rs#B`.
    pub place: String,
    pub title: String,
    /// The folder it's in, or the file a type is in.
    pub folder: String,
    pub rect: Rect,
    /// Read by the index, not only placed by its folder.
    pub read: bool,
    pub functions: usize,
}

/// What flows between two regions, each way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub a: usize,
    pub b: usize,
    /// Labels from `a` to `b`, and back.
    pub forward: Vec<String>,
    pub backward: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct DomainLevel {
    pub regions: Vec<Region>,
    pub links: Vec<Link>,
    /// Jig's own grouping, not yet indexed.
    pub draft: bool,
    /// Each member's region, by place.
    by_place: HashMap<String, usize>,
}

impl DomainLevel {
    /// `domains` over `project`, laid out.
    pub fn of(project: &Project, domains: &Domains, draft: bool) -> DomainLevel {
        let mut regions: Vec<Region> = domains
            .domains
            .iter()
            .enumerate()
            .map(|(ix, domain)| Region {
                key: format!("domain:{}", domain.name),
                kind: RegionKind::Domain(ix),
                name: domain.name.clone(),
                purpose: domain.purpose.clone(),
                offers: domain.offers.clone(),
                why: domain.why.clone(),
                members: Vec::new(),
                rect: Rect::default(),
                weight: 0,
                files: 0,
                functions: 0,
            })
            .collect();
        let shared = regions.len();
        regions.push(Region {
            key: "domain:/shared".into(),
            kind: RegionKind::Shared,
            name: "Shared".into(),
            purpose: "Code used everywhere.".into(),
            offers: Vec::new(),
            why: String::new(),
            members: Vec::new(),
            rect: Rect::default(),
            weight: 0,
            files: 0,
            functions: 0,
        });
        regions.push(Region {
            key: "domain:/unsorted".into(),
            kind: RegionKind::Unsorted,
            name: "Unsorted".into(),
            purpose: "New files no domain holds yet. Update Domains places them.".into(),
            offers: Vec::new(),
            why: String::new(),
            members: Vec::new(),
            rect: Rect::default(),
            weight: 0,
            files: 0,
            functions: 0,
        });
        let region_of = |home: Home| match home {
            Home::Domain(ix) => ix,
            Home::Shared => shared,
            Home::Unsorted => shared + 1,
        };
        let read: HashMap<&std::path::Path, usize> = project
            .files
            .iter()
            .enumerate()
            .map(|(ix, file)| (file.path.as_path(), ix))
            .collect();
        // Functions by file, and by type in it, with how much code each
        // type's functions take.
        let mut functions: HashMap<(usize, Option<&str>), (usize, u64)> = HashMap::new();
        for function in &project.functions {
            let size = function.range().len() as u64;
            let entry = functions.entry((function.file, None)).or_default();
            entry.0 += 1;
            if let Some(owner) = &function.outline.owner {
                let entry = functions
                    .entry((function.file, Some(owner.as_str())))
                    .or_default();
                entry.0 += 1;
                entry.1 += size;
            }
        }
        for path in all_files(project) {
            let place = place_of(&path);
            let home = domains.home_of(&place);
            let file = read.get(path.as_path()).copied();
            let mut weight = file
                .and_then(|ix| project.files[ix].stamp)
                .map(|(_, len)| len)
                .or_else(|| {
                    std::fs::metadata(project.root.join(&path))
                        .ok()
                        .map(|m| m.len())
                })
                .unwrap_or(0);
            let mut count = file
                .and_then(|ix| functions.get(&(ix, None)))
                .map_or(0, |(n, _)| *n);
            // Types with a home of their own are members there.
            if let Some(ix) = file {
                for ty in &project.files[ix].types {
                    let typed = format!("{place}#{}", ty.name);
                    let own = domains.home_of(&typed);
                    if own == home {
                        continue;
                    }
                    let (n, size) = functions
                        .get(&(ix, Some(ty.name.as_str())))
                        .copied()
                        .unwrap_or_default();
                    weight = weight.saturating_sub(size);
                    count = count.saturating_sub(n);
                    let region = &mut regions[region_of(own)];
                    region.weight += size;
                    region.functions += n;
                    region.members.push(Member {
                        key: format!("member:{typed}"),
                        place: typed,
                        title: ty.name.clone(),
                        folder: file_name(&place),
                        rect: Rect::default(),
                        read: true,
                        functions: n,
                    });
                }
            }
            let region = &mut regions[region_of(home)];
            region.weight += weight;
            region.functions += count;
            region.files += 1;
            region.members.push(Member {
                key: format!("member:{place}"),
                title: file_name(&place),
                folder: place
                    .rsplit_once('/')
                    .map_or("", |(dir, _)| dir)
                    .to_string(),
                place,
                rect: Rect::default(),
                read: file.is_some(),
                functions: count,
            });
        }
        for region in &mut regions {
            region
                .members
                .sort_by(|a, b| a.folder.cmp(&b.folder).then(a.place.cmp(&b.place)));
        }
        let mut links: Vec<Link> = Vec::new();
        for flow in &domains.flows {
            let (Some(from), Some(to)) = (domains.index_of(&flow.from), domains.index_of(&flow.to))
            else {
                continue;
            };
            if from == to {
                continue;
            }
            let (a, b) = (from.min(to), from.max(to));
            let at = match links.iter().position(|l| l.a == a && l.b == b) {
                Some(at) => at,
                None => {
                    links.push(Link {
                        a,
                        b,
                        forward: Vec::new(),
                        backward: Vec::new(),
                    });
                    links.len() - 1
                }
            };
            let label = flow.label.clone();
            if from == a {
                links[at].forward.push(label);
            } else {
                links[at].backward.push(label);
            }
        }
        let mut level = DomainLevel {
            regions,
            links,
            draft,
            by_place: HashMap::new(),
        };
        level.place_regions();
        level.by_place = level
            .regions
            .iter()
            .enumerate()
            .flat_map(|(ix, region)| region.members.iter().map(move |m| (m.place.clone(), ix)))
            .collect();
        level
    }

    /// The region `place` is in.
    pub fn region_of(&self, place: &str) -> Option<usize> {
        self.by_place.get(place).copied()
    }

    /// The regions drawn: the domains, and Shared and Unsorted when they
    /// hold anything.
    pub fn showing(&self) -> impl Iterator<Item = (usize, &Region)> {
        self.regions.iter().enumerate().filter(|(_, region)| {
            matches!(region.kind, RegionKind::Domain(_)) || !region.members.is_empty()
        })
    }

    /// Everything drawn.
    pub fn bounds(&self) -> Option<Rect> {
        self.showing()
            .map(|(_, region)| region.rect)
            .reduce(|a, b| a.union(&b))
    }

    pub fn unsorted(&self) -> Vec<String> {
        self.regions
            .iter()
            .filter(|region| region.kind == RegionKind::Unsorted)
            .flat_map(|region| region.members.iter().map(|m| m.place.clone()))
            .collect()
    }

    /// Each region sized by its code and members, the domains packed in
    /// rows and Shared and Unsorted below them, apart.
    fn place_regions(&mut self) {
        let heaviest = self
            .regions
            .iter()
            .filter(|region| matches!(region.kind, RegionKind::Domain(_)))
            .map(|region| region.weight)
            .max()
            .unwrap_or(1)
            .max(1);
        for region in &mut self.regions {
            let by_code = match region.kind {
                RegionKind::Domain(_) => {
                    let share = (region.weight as f32 / heaviest as f32).sqrt();
                    MIN_WIDTH + (MAX_WIDTH - MIN_WIDTH) * share
                }
                _ => MIN_WIDTH * 0.75,
            };
            let count = region.members.len().max(1) as f32;
            let columns_fit = (ASPECT * count * (MEMBER_HEIGHT + GAP) / (MEMBER_WIDTH + GAP))
                .sqrt()
                .ceil();
            let width = by_code.max(columns_fit * (MEMBER_WIDTH + GAP) - GAP + 2. * PAD);
            let columns = ((width - 2. * PAD + GAP) / (MEMBER_WIDTH + GAP))
                .floor()
                .max(1.) as usize;
            for (ix, member) in region.members.iter_mut().enumerate() {
                member.rect = Rect {
                    x: PAD + (ix % columns) as f32 * (MEMBER_WIDTH + GAP),
                    y: HEADER + (ix / columns) as f32 * (MEMBER_HEIGHT + GAP),
                    width: MEMBER_WIDTH,
                    height: MEMBER_HEIGHT,
                };
            }
            let rows = region.members.len().div_ceil(columns) as f32;
            let grid = (rows * (MEMBER_HEIGHT + GAP) - GAP).max(0.);
            let height = (HEADER + grid + PAD).max(width / ASPECT * 0.7);
            region.rect = Rect {
                x: 0.,
                y: 0.,
                width,
                height,
            };
        }
        // The domains in rows, as near `ASPECT` as they pack.
        let domains: Vec<usize> = (0..self.regions.len())
            .filter(|&ix| matches!(self.regions[ix].kind, RegionKind::Domain(_)))
            .collect();
        let area: f32 = domains
            .iter()
            .map(|&ix| {
                let r = self.regions[ix].rect;
                (r.width + REGION_GAP) * (r.height + REGION_GAP)
            })
            .sum();
        let widest = domains
            .iter()
            .map(|&ix| self.regions[ix].rect.width)
            .fold(0., f32::max);
        let row = (area * ASPECT).sqrt().max(widest);
        let (mut x, mut y, mut tallest, mut right) = (0f32, 0f32, 0f32, 0f32);
        for &ix in &domains {
            let rect = &mut self.regions[ix].rect;
            if x > 0. && x + rect.width > row {
                x = 0.;
                y += tallest + REGION_GAP;
                tallest = 0.;
            }
            rect.x = x;
            rect.y = y;
            x += rect.width + REGION_GAP;
            right = right.max(x - REGION_GAP);
            tallest = tallest.max(rect.height);
        }
        // Shared and Unsorted apart, below.
        let mut x = 0.;
        let below = if domains.is_empty() {
            0.
        } else {
            y + tallest + APART
        };
        for region in &mut self.regions {
            if matches!(region.kind, RegionKind::Domain(_)) || region.members.is_empty() {
                continue;
            }
            region.rect.x = x;
            region.rect.y = below;
            x += region.rect.width + REGION_GAP;
        }
        // Members where their region is.
        for region in &mut self.regions {
            let (rx, ry) = (region.rect.x, region.rect.y);
            for member in &mut region.members {
                member.rect.x += rx;
                member.rect.y += ry;
            }
        }
        let _ = right;
    }
}

fn file_name(place: &str) -> String {
    place
        .split('#')
        .next()
        .unwrap_or(place)
        .rsplit('/')
        .next()
        .unwrap_or(place)
        .to_string()
}

fn plural(count: usize, what: &str) -> String {
    format!("{count} {what}{}", if count == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::super::super::index::tests::fixture;
    use super::*;

    fn domains() -> Domains {
        toml::from_str(
            r#"
[[domain]]
name = "Bug Reports"
purpose = "Report what's broken."
members = ["server/src/org/demo/bugs/", "server/src/org/demo/users/UserService.java#UserService"]

[[domain]]
name = "Rooms"
purpose = "Meet in rooms."
members = ["client/src/rooms.ts"]

[[flow]]
from = "Rooms"
to = "Bug Reports"
label = "reports problems to"

[[flow]]
from = "Bug Reports"
to = "Rooms"
label = "closes rooms in"

[shared]
members = ["client/src/net/"]
"#,
        )
        .unwrap()
    }

    #[test]
    fn regions_hold_their_members_and_shared_and_unsorted_sit_apart() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let level = DomainLevel::of(&project, &domains(), false);
        let names: Vec<&str> = level.showing().map(|(_, r)| r.name.as_str()).collect();
        assert_eq!(names, ["Bug Reports", "Rooms", "Shared", "Unsorted"]);
        let bugs = &level.regions[0];
        // Its files, and a type with a home of its own.
        assert_eq!(bugs.files, 3);
        assert!(
            bugs.members
                .iter()
                .any(|m| m.place == "server/src/org/demo/users/UserService.java#UserService")
        );
        // UserService.java itself is named by nothing.
        assert_eq!(
            level.unsorted(),
            ["server/src/org/demo/users/UserService.java"]
        );
        // Members inside their region, below its header.
        for region in &level.regions {
            for member in &region.members {
                assert!(member.rect.right() <= region.rect.right() + 0.01);
                assert!(member.rect.bottom() <= region.rect.bottom() + 0.01);
                assert!(member.rect.y >= region.rect.y + HEADER);
            }
        }
        // Shared and Unsorted below every domain.
        let lowest = level.regions[..2]
            .iter()
            .map(|r| r.rect.bottom())
            .fold(0., f32::max);
        assert!(level.regions[2].rect.y > lowest);
        assert!(!level.regions[0].rect.overlaps(&level.regions[1].rect));
    }

    #[test]
    fn one_link_per_pair_carries_both_ways() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let level = DomainLevel::of(&project, &domains(), false);
        assert_eq!(
            level.links,
            [Link {
                a: 0,
                b: 1,
                forward: vec!["closes rooms in".into()],
                backward: vec!["reports problems to".into()],
            }]
        );
    }

    #[test]
    fn more_code_makes_a_bigger_region() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let level = DomainLevel::of(&project, &domains(), false);
        assert!(level.regions[0].weight > level.regions[1].weight);
        assert!(level.regions[0].rect.width > level.regions[1].rect.width);
    }
}
