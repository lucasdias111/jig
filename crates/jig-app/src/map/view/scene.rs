//! What the canvas draws, the same way for every level: frames, boxes on
//! them and arrows between, in the level's drawing. A Plan's scene is its
//! boxes as laid out. Domain's is its regions and the links between them;
//! Site's is the same regions, each opening onto its share of the code as
//! it grows on screen, every box inside opening the same way once what's
//! in it would read, and the calls between boxes side by side.

use std::rc::Rc;
use std::time::Instant;

use super::super::domains::Home;
use super::super::domains::level::RegionKind;
use super::super::layout::{HEADER_HEIGHT, Placed, Rect};
use super::super::plan::Item;
use super::super::site::{Calls, Kind, Site};
use super::domain::picked_link;
use super::zoom::Camera;
use super::zoom::FADE_OUT;
use super::{Level, MapView};

/// More arrows than this at Site, and only those with the most calls.
const MAX_ARROWS: usize = 160;

/// How a box or frame looks, by what it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Look {
    Function,
    /// An interface's or trait's method, without a body.
    Abstract,
    External,
    More,
    Gone,
    Folder,
    Package,
    File,
    /// A file the index can't read, placed by its folder.
    Unread,
    Type,
    /// A Plan's frame: a type or a file its functions are in.
    Owner,
    /// A file as a whole at Plan: one the index reads, or one it can't.
    Source,
    UnreadSource,
}

impl Look {
    /// Whether its words are drawn at one size on screen, however far in
    /// the view is: Site's, which must read at any zoom.
    pub fn fixed_text(self) -> bool {
        !matches!(
            self,
            Look::Function
                | Look::Abstract
                | Look::External
                | Look::More
                | Look::Gone
                | Look::Owner
                | Look::Source
                | Look::UnreadSource
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub key: String,
    pub rect: Rect,
    pub title: String,
    /// A signature, or a purpose.
    pub detail: String,
    /// How much it holds, or what it calls.
    pub note: String,
    pub badges: Vec<String>,
    pub look: Look,
    pub alpha: f32,
    /// A Plan's own function.
    pub focus: bool,
    /// Callers, callees and implementations not drawn, to open.
    pub hidden: [usize; 3],
    pub unknown_target: bool,
    /// Its calls checked with the language server.
    pub checked: bool,
    /// The saved domain it's tinted with.
    pub tint: Option<usize>,
    /// Which box it is on its level: a Plan's node, a Site's, or at Domain
    /// the region it's in.
    pub ix: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameBox {
    pub key: String,
    pub rect: Rect,
    /// Its title bar's height, in the drawing.
    pub header: f32,
    pub title: String,
    pub detail: String,
    pub note: String,
    pub look: Look,
    pub alpha: f32,
    pub tint: Option<usize>,
    pub ix: usize,
}

/// A domain at Domain, or Shared or Unsorted: a big region with its name,
/// purpose and offers, its files inside.
#[derive(Clone, Debug, PartialEq)]
pub struct RegionBox {
    pub key: String,
    pub rect: Rect,
    /// Where its name, purpose and offers go, above what's inside.
    pub header: f32,
    pub name: String,
    pub purpose: String,
    pub offers: Vec<String>,
    pub why: String,
    /// How much it holds.
    pub note: String,
    pub kind: RegionKind,
    /// One of Jig's drafts.
    pub draft: bool,
    /// Proposed by the agent, not yet kept.
    pub proposed: bool,
    /// How far its files show, from 0 to 1.
    pub open: f32,
    /// The domain picked: selected, or holding the box selected.
    pub lit: bool,
    pub alpha: f32,
    /// Its place in the level.
    pub ix: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arrow {
    pub from: (f32, f32),
    pub to: (f32, f32),
    /// The boxes it joins, by key.
    pub keys: (String, String),
    /// Across from a box's right to another's left (a Plan's), or bowed
    /// between their edges (Site's).
    pub across: bool,
    pub dashed: bool,
    /// The calls it stands for, at Site.
    pub calls: Option<Calls>,
    pub alpha: f32,
    /// Its heads, at where it leaves and where it arrives.
    pub heads: (bool, bool),
    /// What flows along it at Domain: forward (from where it leaves) or
    /// back, and in words.
    pub labels: Vec<(bool, String)>,
    /// A link between two domains, drawn steady whether it's labelled or
    /// not.
    pub link: bool,
    /// Its words drawn faintly: a link of a domain not selected.
    pub faint: bool,
    /// A link of the selected domain: drawn stronger, in its colour
    /// (`tint`), its words headed by the domain at the other end (`title`,
    /// in that domain's colour, `title_tint`) and read from the selected
    /// domain's side: forward is what goes out of it.
    pub picked: bool,
    pub tint: Option<usize>,
    pub title: Option<String>,
    pub title_tint: Option<usize>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub regions: Vec<RegionBox>,
    pub frames: Vec<FrameBox>,
    pub cards: Vec<Card>,
    pub arrows: Vec<Arrow>,
}

impl Scene {
    /// The box or frame under drawing point `at`, innermost.
    pub fn card_at(&self, at: (f32, f32)) -> Option<&Card> {
        self.cards.iter().rev().find(|card| card.rect.contains(at))
    }

    pub fn frame_at(&self, at: (f32, f32)) -> Option<&FrameBox> {
        self.frames
            .iter()
            .rev()
            .find(|frame| frame.rect.contains(at))
    }
}

impl MapView {
    /// The level showing as drawn by `camera` now.
    pub(super) fn scene(&self, camera: Camera, now: Instant) -> Scene {
        match self.level {
            Level::Plan(_) => self.plan_scene(&self.drawn_plan(now), now),
            Level::Site => self.site_scene(camera),
            Level::Domain => self.domain_scene(camera),
        }
    }

    /// The level showing as it's drawn now, to fade out as another comes.
    pub(super) fn scene_now(&self) -> Scene {
        self.scene(self.camera(), Instant::now())
    }

    fn plan_scene(&self, placed: &Placed, now: Instant) -> Scene {
        let mut scene = Scene::default();
        let project = self.project.as_ref();
        for (frame, rect) in &placed.frames {
            let title = self.plan.frames[*frame].title.clone();
            scene.frames.push(FrameBox {
                key: format!("frame:{}", self.plan.frames[*frame].key),
                rect: *rect,
                title,
                detail: String::new(),
                note: String::new(),
                look: Look::Owner,
                alpha: 1.,
                tint: None,
                ix: *frame,
                header: HEADER_HEIGHT,
            });
        }
        for (ix, node) in self.plan.nodes.iter().enumerate() {
            let look = match node.item {
                Item::Function(id) => {
                    if project.is_some_and(|p| p.functions[id].outline.is_abstract) {
                        Look::Abstract
                    } else {
                        Look::Function
                    }
                }
                Item::External(_) => Look::External,
                Item::More { .. } => Look::More,
                Item::Gone(_) => Look::Gone,
                Item::File { read: true, .. } => Look::Source,
                Item::File { read: false, .. } => Look::UnreadSource,
            };
            scene.cards.push(Card {
                key: node.key.clone(),
                rect: placed.nodes[ix],
                title: node.title.clone(),
                detail: node.detail.clone(),
                note: node.note.clone(),
                badges: node.badges.clone(),
                look,
                alpha: self.arriving(&node.key, now),
                focus: self.plan.focus == Some(ix),
                hidden: [
                    node.hidden_callers,
                    node.hidden_callees,
                    node.implementations,
                ],
                unknown_target: node.unknown_target,
                checked: node
                    .function()
                    .is_some_and(|id| project.is_some_and(|p| p.checked(id))),
                tint: self.plan_tint(&node.item),
                ix,
            });
        }
        for wire in &self.plan.wires {
            let (from, to) = (placed.nodes[wire.from], placed.nodes[wire.to]);
            scene.arrows.push(Arrow {
                from: (from.right(), from.y + HEADER_HEIGHT / 2.),
                to: (to.x, to.y + HEADER_HEIGHT / 2.),
                keys: (
                    self.plan.nodes[wire.from].key.clone(),
                    self.plan.nodes[wire.to].key.clone(),
                ),
                across: true,
                dashed: wire.kind.dashed(),
                calls: None,
                heads: (false, false),
                labels: Vec::new(),
                link: false,
                faint: false,
                picked: false,
                tint: None,
                title: None,
                title_tint: None,
                alpha: self
                    .arriving(&self.plan.nodes[wire.from].key, now)
                    .min(self.arriving(&self.plan.nodes[wire.to].key, now)),
            });
        }
        scene
    }

    /// Site as `camera` sees it: the regions, those grown big enough open
    /// onto their share of the code, every box inside showing as a frame
    /// with what's inside fading in once that would read; the arrows
    /// between boxes side by side, calls leaving a region as stubs at its
    /// edge, and the domains' flows between regions still closed.
    fn site_scene(&self, camera: Camera) -> Scene {
        let mut scene = Scene::default();
        let (Some(site), Some(level)) = (self.site.as_ref(), self.domain_level()) else {
            return scene;
        };
        let (width, height) = self.canvas_size();
        let view = Rect {
            x: camera.x - width / 2. / camera.zoom,
            y: camera.y - height / 2. / camera.zoom,
            width: width / camera.zoom,
            height: height / camera.zoom,
        };
        let zoom = camera.zoom;
        let tints = self.has_tints() && !self.domains.tints_off;
        // How opaque each box is: what's inside a box opening fades in.
        let mut alpha = vec![0f32; site.nodes.len()];
        let mut stack: Vec<(usize, f32)> = Vec::new();
        for region in site.regions() {
            let node = &site.nodes[region];
            let Some(at) = level.regions.iter().position(|r| r.key == node.key) else {
                continue;
            };
            let open = node.opening(zoom);
            scene.regions.push(self.region_box(&level, at, open));
            alpha[region] = 1.;
            if open > 0. && node.rect.overlaps(&view) {
                for &child in node.children.iter().rev() {
                    stack.push((child, open));
                }
            }
        }
        while let Some((ix, shown)) = stack.pop() {
            let node = &site.nodes[ix];
            alpha[ix] = shown;
            if !node.rect.overlaps(&view) {
                continue;
            }
            let open = node.opening(zoom);
            let look = match node.kind {
                Kind::Component => Look::Package,
                Kind::Folder | Kind::Root | Kind::Region(_) => Look::Folder,
                Kind::File { read: true } => Look::File,
                Kind::File { read: false } => Look::Unread,
                Kind::Type => Look::Type,
            };
            let tint = match site.nodes[node.region].kind {
                Kind::Region(Home::Domain(domain)) if tints => Some(domain),
                _ => None,
            };
            let about = node.purpose.clone().unwrap_or_else(|| {
                if matches!(node.kind, Kind::Folder | Kind::Component) {
                    node.contents.clone()
                } else {
                    String::new()
                }
            });
            if open > 0. {
                scene.frames.push(FrameBox {
                    key: node.key.clone(),
                    rect: node.rect,
                    title: node.title.clone(),
                    detail: node.purpose.clone().unwrap_or_default(),
                    note: node.detail.clone(),
                    look,
                    alpha: shown * open,
                    tint,
                    ix,
                    header: node.header,
                });
                for &child in node.children.iter().rev() {
                    stack.push((child, shown * open));
                }
            }
            if open < 1. {
                scene.cards.push(Card {
                    key: node.key.clone(),
                    rect: node.rect,
                    title: node.title.clone(),
                    detail: about,
                    note: node.detail.clone(),
                    badges: Vec::new(),
                    look,
                    alpha: shown * (1. - open),
                    focus: false,
                    hidden: [0; 3],
                    unknown_target: false,
                    checked: false,
                    tint,
                    ix,
                });
            }
        }
        // Inner boxes after outer ones, so they're drawn over them.
        scene.cards.sort_by_key(|card| card.ix);
        scene.frames.sort_by_key(|frame| frame.ix);
        // Arrows join the boxes showing, as a box is counted open half way.
        let shown = site.shown_as(|ix| site.nodes[ix].opening(zoom) >= 0.5);
        let arrows = self.site_arrows(site, &shown);
        for arrow in arrows
            .iter()
            .filter(|arrow| {
                let (a, b) = (site.nodes[arrow.from].rect, site.nodes[arrow.to].rect);
                a.overlaps(&view) || b.overlaps(&view)
            })
            .take(MAX_ARROWS)
        {
            let (a, b) = (site.nodes[arrow.from].rect, site.nodes[arrow.to].rect);
            let (from, to) = edge_points(a, b);
            scene.arrows.push(Arrow {
                from,
                to,
                keys: (
                    site.nodes[arrow.from].key.clone(),
                    site.nodes[arrow.to].key.clone(),
                ),
                across: false,
                dashed: false,
                calls: Some(arrow.calls.clone()),
                alpha: alpha[arrow.from].min(alpha[arrow.to]).max(0.25),
                heads: (false, true),
                labels: Vec::new(),
                link: false,
                faint: false,
                picked: false,
                tint: None,
                title: None,
                title_tint: None,
            });
        }
        // Between regions, the domains' links, one per pair; what flows
        // along them only on the selected domain's.
        let picked = self.selected_region();
        let mut links: Vec<Arrow> = level
            .links
            .iter()
            .map(|link| {
                let mut arrow = picked_link(&level, link, picked.as_deref());
                if !arrow.picked {
                    arrow.labels.clear();
                }
                arrow
            })
            .collect();
        links.sort_by_key(|arrow| arrow.picked);
        scene.arrows.extend(links);
        scene
    }

    /// The arrows between the Site boxes `shown`, worked out again only
    /// when which boxes show changes.
    fn site_arrows(&self, site: &Site, shown: &[usize]) -> super::SiteArrows {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        shown.hash(&mut hasher);
        (site as *const Site as usize).hash(&mut hasher);
        let key = hasher.finish();
        if let Some((at, arrows)) = self.site_arrows.borrow().as_ref()
            && *at == key
        {
            return arrows.clone();
        }
        let arrows = Rc::new(site.arrows(shown));
        *self.site_arrows.borrow_mut() = Some((key, arrows.clone()));
        arrows
    }

    /// Plan's boxes that left on a re-centring, fading where they were.
    pub(super) fn leaving_scene(&self, now: Instant) -> Option<(Scene, f32)> {
        let (scene, at) = self.leaving.as_ref()?;
        let t = now.duration_since(*at).as_secs_f32() / FADE_OUT;
        (t < 1.).then(|| (scene.clone(), 1. - t))
    }
}

/// Where an arrow from box `a` to box `b` leaves and arrives: on their
/// edges, along the line between their middles.
pub(super) fn edge_points(a: Rect, b: Rect) -> ((f32, f32), (f32, f32)) {
    let (ac, bc) = (a.centre(), b.centre());
    (border(a, bc), border(b, ac))
}

/// Where the line from `rect`'s middle towards `to` leaves it.
pub(super) fn border(rect: Rect, to: (f32, f32)) -> (f32, f32) {
    let (cx, cy) = rect.centre();
    let (dx, dy) = (to.0 - cx, to.1 - cy);
    if dx == 0. && dy == 0. {
        return (cx, cy);
    }
    let sx = if dx == 0. {
        f32::MAX
    } else {
        rect.width / 2. / dx.abs()
    };
    let sy = if dy == 0. {
        f32::MAX
    } else {
        rect.height / 2. / dy.abs()
    };
    let s = sx.min(sy);
    (cx + dx * s, cy + dy * s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_leave_from_the_edges() {
        let a = Rect {
            x: 0.,
            y: 0.,
            width: 100.,
            height: 50.,
        };
        let b = Rect { x: 300., ..a };
        assert_eq!(edge_points(a, b), ((100., 25.), (300., 25.)));
    }
}
