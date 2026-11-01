//! What the canvas draws, the same way for every level: frames, boxes on
//! them and arrows between, in the level's drawing. A Plan's scene is its
//! boxes as laid out. Domain's is its regions and the links between them;
//! Site's is the same regions, each opening onto its share of the code as
//! it grows on screen, every box inside opening the same way once what's
//! in it would read, and the calls between boxes side by side.

use std::rc::Rc;
use std::time::Instant;

use super::domain::picked_link;
use super::zoom::Camera;
use super::zoom::FADE_OUT;
use super::{Level, MapView};
use crate::domain::domains::Home;
use crate::domain::domains::level::RegionKind;
use crate::domain::layout::{HEADER_HEIGHT, Placed, Rect};
use crate::domain::plan::Item;
use crate::domain::site::{Calls, Kind, Site};
mod site;

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
    /// At Site, an arrow of the box selected (or of the box standing for
    /// it): drawn stronger, the others fainter.
    pub lit: Option<bool>,
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
                lit: None,
                alpha: self
                    .arriving(&self.plan.nodes[wire.from].key, now)
                    .min(self.arriving(&self.plan.nodes[wire.to].key, now)),
            });
        }
        scene
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
mod tests;
