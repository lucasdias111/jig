//! The mouse and keys on the Map. Click selects, Shift-click or a box
//! several; dragging moves a Plan's boxes (and only moves them), right or
//! middle drag pans, double-click enters a box. At Domain, dropping a
//! domain onto another merges them; at Site, with the domains' tints
//! showing, a file dragged into another domain's region moves to it. Double-clicking a
//! domain's name renames it. Resting on a Plan's function peeks at its
//! code; on a Site arrow, lists its calls. With a sketching tool picked
//! (B, A, N, F), a press draws instead (`view::sketch`), and a sketch's
//! shapes are selected, moved and edited like boxes. Keys: ⌘↑ out, ⌘↓ or
//! Enter in, ⌘A all, ⌘Z and ⇧⌘Z undo and redo edits to sketches and
//! domains, ⌫ deletes the sketch shapes selected, F2 renames the
//! domain selected, ⌘↩ keeps a proposal, ←/→ step through the agent's
//! trace, Tab or Enter keeps the agent's descriptions, Esc ends a tour,
//! rejects descriptions, clears the selection (or discards a proposal) and
//! then goes back to the code.

use std::time::Instant;

use gpui_kit::*;

use super::domain::Field;
use super::{Level, MapEvent, MapView, site_key};
use crate::domain::domains::level::RegionKind;
use crate::domain::layout::Rect;
use crate::domain::plan::Item;
use crate::store::cache::Folder;
mod hover;
mod keys;
mod mouse;
pub(super) use hover::*;

/// How far, in screen points, the pointer moves before a press drags.
const DRAG_START: f32 = 4.;
/// How long the pointer rests on a function before its code shows.
pub(super) const PEEK_MS: u128 = 500;
/// Lines of a function's code a peek shows.
const PEEK_LINES: usize = 14;
/// How near an arrow, in screen points, the pointer is on it.
const ARROW_REACH: f32 = 6.;

pub(super) enum Drag {
    /// Pressed on box `key`, not moved far yet.
    Press {
        key: String,
        start: Point<Pixels>,
        shift: bool,
    },
    /// Moving the selected Plan boxes from where each was, the pointer
    /// having started at `start` in the drawing.
    Nodes {
        start: (f32, f32),
        from: Vec<(String, (f32, f32))>,
    },
    Marquee {
        from: Point<Pixels>,
        to: Point<Pixels>,
        kept: Vec<String>,
    },
    Pan {
        last: Point<Pixels>,
        moved: bool,
    },
    /// A file or type carried to another domain.
    Member {
        place: String,
        title: String,
        at: Point<Pixels>,
    },
    /// A domain, by its region, carried onto another to merge.
    Region {
        region: usize,
        at: Point<Pixels>,
    },
    /// Drawing with a tool, or on a sketch's shape.
    Sketch(super::sketch::SketchDrag),
}

/// What the pointer is over.
pub(super) enum Hover {
    /// A box, by key, since when.
    Card(String, Instant),
    /// A Site arrow, by its place in the scene drawn, and where.
    Arrow(usize, Point<Pixels>),
}

impl MapView {
    /// The Plan box under `position`.
    pub(super) fn plan_box_at(&self, position: Point<Pixels>) -> Option<usize> {
        let at = self.to_drawing(position);
        (0..self.placed.nodes.len())
            .rev()
            .find(|&ix| self.placed.nodes[ix].contains(at))
    }

    /// The innermost Site box showing under `position`: a box, else the
    /// open frame it's in.
    pub(super) fn site_box_at(&self, position: Point<Pixels>) -> Option<usize> {
        let at = self.to_drawing(position);
        let drawn = self.drawn.borrow();
        drawn
            .card_at(at)
            .map(|card| card.ix)
            .or_else(|| drawn.frame_at(at).map(|frame| frame.ix))
            .or_else(|| {
                // Not drawn yet: the innermost box there.
                let site = self.site.as_ref()?;
                (0..site.nodes.len())
                    .rev()
                    .find(|&ix| site.nodes[ix].rect.contains(at))
            })
    }

    /// The key a box under `position` is selected by.
    fn key_at(&self, position: Point<Pixels>) -> Option<String> {
        match self.level {
            Level::Plan(_) => self
                .plan_box_at(position)
                .map(|ix| self.plan.nodes[ix].key.clone()),
            Level::Site => self
                .site_box_at(position)
                .filter(|&ix| ix != 0)
                .map(|ix| site_key(&self.site_nodes()[ix].key)),
            Level::Domain => self.domain_key_at(position),
        }
    }

    pub(super) fn selected_plan(&self) -> Vec<usize> {
        let mut found: Vec<usize> = self
            .selected
            .iter()
            .filter_map(|key| self.plan.index_of(key))
            .collect();
        found.sort();
        found
    }

    fn select(&mut self, key: String, add: bool) {
        if add {
            match self.selected.iter().position(|k| *k == key) {
                Some(at) => {
                    self.selected.remove(at);
                }
                None => self.selected.push(key),
            }
        } else {
            self.selected = vec![key];
        }
    }

    /// Region `key`'s place in the Domain level.
    pub(super) fn region_by_key(&self, key: &str) -> Option<usize> {
        self.domain_level()?
            .regions
            .iter()
            .position(|region| region.key == key)
    }

    pub(super) fn pinch(&mut self, event: &PinchEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.phase == TouchPhase::Ended {
            self.zoom_lock = None;
            return;
        }
        self.zoom_at(1. + event.delta, event.position, cx);
    }
}

pub(super) fn far(a: Point<Pixels>, b: Point<Pixels>) -> bool {
    far_by(a, b, DRAG_START)
}

/// Whether `a` and `b` are more than `by` screen points apart.
pub(super) fn far_by(a: Point<Pixels>, b: Point<Pixels>, by: f32) -> bool {
    let d = a - b;
    f32::from(d.x).hypot(f32::from(d.y)) > by
}
