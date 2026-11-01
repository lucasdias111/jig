//! Drawing the agent's drawings over the level showing, in the agent's
//! colour: each box a place lands on outlined; a trace's stops numbered
//! with their step ranges and joined by one dashed path; a note or a
//! one-step stop's words under its box; a group framed with its label;
//! ghosts beside the place they go by, each adoptable into the user's
//! sketch; proposed purposes and labels to
//! keep or not. Boxes out of view get an arrow at the canvas edge that
//! flies there. A list in the corner names every drawing and its steps,
//! greying those this level can't show; a tour shows where it is below.

use std::collections::HashMap;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_commands::surface::agent_accent;

use super::draw::{Line, View};
use super::lens::{Landing, Source, Stop, fold};
use super::{MapEvent, MapView};
use crate::domain::layout::Rect;
use crate::domain::lens::{Drawing, Kind, Place};
mod legend;
mod overlay;
mod tour;
use legend::*;
pub(super) use overlay::*;

/// Most lines of words under one box before "+N more".
const MOST_WORDS: usize = 3;
/// How far from the canvas edge an arrow to a box out of view sits.
const EDGE: f32 = 22.;
/// Where the list of drawings and the tour's guide sit, and how wide the
/// list is.
const LIST_TOP: f32 = 12.;
const LIST_WIDTH: f32 = 360.;
/// A ghost box's size on screen.
pub(super) const GHOST: (f32, f32) = (190., 54.);

/// What the user chose for a proposal's ghosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalChoice {
    MakeTask,
    Implement,
    /// Made the user's sketch, all of it.
    Adopt,
    Dismiss,
}

/// A box the agent's drawings land on, on screen.
pub(super) struct Marked {
    pub(super) key: String,
    /// In the drawing, to fly to.
    pub(super) drawn: Rect,
    pub(super) rect: Rect,
    /// Lines of words under it.
    pub(super) words: Vec<String>,
    /// Step ranges, top left.
    pub(super) badges: Vec<String>,
    /// The tour's step is here.
    pub(super) toured: bool,
    /// Its words are proposed, to keep or not.
    pub(super) proposed: bool,
    /// Only pins land here: drawn fainter.
    pub(super) pinned: bool,
    /// A frame holding other boxes (a Plan's type or file): its words go
    /// above it, clear of theirs.
    pub(super) above: bool,
}

/// A path along a trace, or an arrow lit, labelled or proposed.
pub(super) struct Path {
    pub(super) from: (f32, f32),
    pub(super) to: (f32, f32),
    pub(super) dashed: bool,
    pub(super) width: f32,
    pub(super) pinned: bool,
}

/// Words on an arrow, at its middle.
pub(super) struct Tag {
    pub(super) at: (f32, f32),
    pub(super) text: String,
}

/// Places circled together.
pub(super) struct Group {
    pub(super) rect: Rect,
    pub(super) label: String,
    pub(super) pinned: bool,
}

/// A box the agent proposes.
pub(super) struct GhostBox {
    pub(super) rect: Rect,
    pub(super) name: String,
    pub(super) purpose: String,
    /// The conversation's proposal it's in, by id, to adopt it from.
    pub(super) proposal: Option<usize>,
}

/// Everything the agent drew at the level showing, on screen: what the
/// canvas draws, and Export Image.
#[derive(Default)]
pub(super) struct Picture {
    pub(super) boxes: Vec<Marked>,
    pub(super) paths: Vec<Path>,
    pub(super) tags: Vec<Tag>,
    pub(super) groups: Vec<Group>,
    pub(super) ghosts: Vec<GhostBox>,
    /// The conversation's proposals, by id, and their first ghost: where
    /// what to do with them goes.
    pub(super) proposals: Vec<(usize, Rect)>,
    /// A tour is under way: its step's box stands out, the rest fade.
    pub(super) touring: bool,
}

impl Picture {
    fn mark(
        &mut self,
        landing: &Landing,
        screen: &dyn Fn(Rect) -> Rect,
        pinned: bool,
    ) -> &mut Marked {
        let at = match self.boxes.iter().position(|b| b.key == landing.key) {
            Some(at) => at,
            None => {
                self.boxes.push(Marked {
                    key: landing.key.clone(),
                    drawn: landing.rect,
                    rect: screen(landing.rect),
                    words: Vec::new(),
                    badges: Vec::new(),
                    toured: false,
                    proposed: false,
                    pinned: true,
                    above: landing.key.starts_with("frame:"),
                });
                self.boxes.len() - 1
            }
        };
        let marked = &mut self.boxes[at];
        marked.pinned &= pinned;
        marked
    }
}

impl MapView {
    /// The agent's drawings at the level showing, `screen` putting a rect of
    /// the drawing on screen.
    pub(super) fn lens_picture(&self, screen: &dyn Fn(Rect) -> Rect) -> Picture {
        let mut picture = Picture::default();
        let tour = self.lens.tour;
        picture.touring = tour.is_some();
        for (source, drawing) in self.drawings_showing() {
            let pinned = matches!(source, Source::Pin(_));
            let landings: Vec<Option<Landing>> = drawing
                .steps
                .iter()
                .map(|step| self.landing(&step.place))
                .collect();
            match drawing.kind {
                Kind::Trace | Kind::Tour => {
                    let stops = fold(&landings);
                    let toured = tour.filter(|t| t.source == source).map(|t| t.step);
                    for stop in &stops {
                        let marked = picture.mark(&stop.landing, screen, pinned);
                        marked.badges.push(stop.range());
                        marked.toured |= toured.is_some_and(|step| stop.steps.contains(&step));
                        let words = match stop.steps.as_slice() {
                            [step] if drawing.steps[*step].label.is_empty() => continue,
                            [step] => format!("{} {}", step + 1, drawing.steps[*step].label),
                            _ => stop_words(drawing, stop),
                        };
                        marked.words.push(words);
                    }
                    for pair in stops.windows(2) {
                        let (a, b) = (screen(pair[0].landing.rect), screen(pair[1].landing.rect));
                        // A box and the frame it's in: one place, no path.
                        if holds(a, b) || holds(b, a) {
                            continue;
                        }
                        let (from, to) = super::scene::edge_points(a, b);
                        picture.paths.push(Path {
                            from,
                            to,
                            dashed: true,
                            width: 2.5,
                            pinned,
                        });
                    }
                }
                Kind::Highlight | Kind::Group => {
                    for landing in landings.iter().flatten() {
                        picture.mark(landing, screen, pinned);
                    }
                    let hull = landings
                        .iter()
                        .flatten()
                        .map(|l| screen(l.rect))
                        .reduce(|a, b| a.union(&b));
                    if let (Kind::Group, Some(hull)) = (drawing.kind, hull) {
                        let pad = 16.;
                        picture.groups.push(Group {
                            rect: Rect {
                                x: hull.x - pad,
                                y: hull.y - pad,
                                width: hull.width + pad * 2.,
                                height: hull.height + pad * 2.,
                            },
                            label: drawing.title.clone(),
                            pinned,
                        });
                    }
                }
                Kind::Note => {
                    for (step, landing) in drawing.steps.iter().zip(&landings) {
                        if let Some(landing) = landing {
                            picture
                                .mark(landing, screen, pinned)
                                .words
                                .push(step.label.clone());
                        }
                    }
                }
                Kind::Describe => {
                    for (step, landing) in drawing.steps.iter().zip(&landings) {
                        if let Some(landing) = landing {
                            let marked = picture.mark(landing, screen, pinned);
                            marked.words.push(format!("Purpose: {}", step.label));
                            marked.proposed |= !pinned;
                        }
                    }
                }
                Kind::Propose => self.picture_ghosts(&mut picture, source, drawing, screen),
            }
            // Arrows lit or labelled between real places.
            if drawing.kind != Kind::Propose {
                for link in &drawing.links {
                    let (Some(a), Some(b)) = (self.landing(&link.from), self.landing(&link.to))
                    else {
                        continue;
                    };
                    if a.key == b.key {
                        continue;
                    }
                    picture.mark(&a, screen, pinned);
                    picture.mark(&b, screen, pinned);
                    let (from, to) = super::scene::edge_points(screen(a.rect), screen(b.rect));
                    picture.paths.push(Path {
                        from,
                        to,
                        dashed: drawing.kind != Kind::Highlight,
                        width: 3.,
                        pinned,
                    });
                    if !link.label.is_empty() {
                        picture.tags.push(Tag {
                            at: ((from.0 + to.0) / 2., (from.1 + to.1) / 2.),
                            text: link.label.clone(),
                        });
                    }
                }
            }
        }
        picture
    }

    /// A proposal's ghosts, each beside the place it goes by, stacked, and
    /// the arrows between them and real places.
    fn picture_ghosts(
        &self,
        picture: &mut Picture,
        source: Source,
        drawing: &Drawing,
        screen: &dyn Fn(Rect) -> Rect,
    ) {
        let mut placed: HashMap<String, Rect> = HashMap::new();
        let proposal = match source {
            Source::Conversation(id) => Some(id),
            Source::Pin(_) => None,
        };
        let rects = self.ghost_rects(drawing);
        for (ghost, rect) in drawing.ghosts.iter().zip(rects) {
            let (Some(near), Some(rect)) = (self.landing(&ghost.near), rect) else {
                continue;
            };
            picture.mark(&near, screen, matches!(source, Source::Pin(_)));
            let near = screen(near.rect);
            // Adopted: the user's box now, drawn as theirs.
            if proposal.is_some_and(|id| {
                self.sketches
                    .adopted
                    .contains_key(&(id, ghost.name.clone()))
            }) {
                continue;
            }
            let rect = screen(rect);
            let rect = Rect {
                width: GHOST.0,
                height: GHOST.1,
                ..rect
            };
            placed.insert(ghost.name.clone(), rect);
            let (from, to) = super::scene::edge_points(near, rect);
            picture.paths.push(Path {
                from,
                to,
                dashed: true,
                width: 1.5,
                pinned: true,
            });
            picture.ghosts.push(GhostBox {
                rect,
                name: ghost.name.clone(),
                purpose: ghost.purpose.clone(),
                proposal,
            });
        }
        let end = |name: &str| -> Option<Rect> {
            placed
                .get(name)
                .copied()
                .or_else(|| self.landing(name).map(|l| screen(l.rect)))
        };
        for link in &drawing.links {
            if let (Some(a), Some(b)) = (end(&link.from), end(&link.to)) {
                let (from, to) = super::scene::edge_points(a, b);
                picture.paths.push(Path {
                    from,
                    to,
                    dashed: true,
                    width: 2.,
                    pinned: false,
                });
                if !link.label.is_empty() {
                    picture.tags.push(Tag {
                        at: ((from.0 + to.0) / 2., (from.1 + to.1) / 2.),
                        text: link.label.clone(),
                    });
                }
            }
        }
        if let (Source::Conversation(id), Some(first)) = (
            source,
            drawing.ghosts.iter().find_map(|g| placed.get(&g.name)),
        ) {
            picture.proposals.push((id, *first));
        }
    }
}

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Whether `outer` holds all of `inner`.
fn holds(outer: Rect, inner: Rect) -> bool {
    outer.x <= inner.x + 0.5
        && outer.y <= inner.y + 0.5
        && outer.right() + 0.5 >= inner.right()
        && outer.bottom() + 0.5 >= inner.bottom()
}
