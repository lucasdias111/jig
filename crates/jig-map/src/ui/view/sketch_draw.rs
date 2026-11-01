//! Drawing the user's sketches, in neutral ink apart from the agent's
//! orange: boxes dashed while they're ideas, solid once they're real code
//! (a "?" when the name could be several things), arrows dashed until the
//! code calls along them, yellow sticky notes, dashed frames. Above each
//! sketch a bar with its title, how much of it is real, and what to do
//! with it: Make Task, its task and Implement, Archive. Elsewhere, a mark
//! on the boxes carrying sketches of other levels. The tool strip sits in
//! the canvas's bottom left; the Sketches list opens from the header.

use gpui_kit::component::input::Input;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::draw::{Line, View, bow_point};
use super::sketch::{EditTarget, SketchDrag, TaskInfo, Tool};
use super::{MapView, input::Drag};
use crate::domain::layout::Rect;
use crate::domain::sketch::{Match, ShapeKind, ShapeRef};
mod bar;
mod shapes;
mod tools;
pub(super) use shapes::*;
pub(super) use tools::*;

/// A box's label, from its height on screen: no smaller than this reads.
const SMALLEST_TEXT: f32 = 7.;

/// How a box of a sketch is drawn.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum BoxState {
    /// An idea: no code yet.
    Sketched,
    /// Real code, at this place.
    Real(String),
    /// Its name could be several things.
    Unsure(usize),
}

pub(super) struct DrawnBox {
    pub(super) key: String,
    pub(super) rect: Rect,
    pub(super) label: String,
    pub(super) purpose: String,
    pub(super) state: BoxState,
    pub(super) selected: bool,
}

pub(super) struct DrawnNote {
    pub(super) key: String,
    pub(super) rect: Rect,
    pub(super) text: String,
    pub(super) selected: bool,
}

pub(super) struct DrawnFrame {
    pub(super) key: String,
    pub(super) rect: Rect,
    pub(super) label: String,
    pub(super) selected: bool,
}

pub(super) struct DrawnArrow {
    pub(super) from: (f32, f32),
    pub(super) to: (f32, f32),
    pub(super) label: String,
    /// The code calls along it.
    pub(super) real: bool,
    pub(super) selected: bool,
}

/// A sketch's bar, above it.
pub(super) struct Bar {
    pub(super) name: String,
    pub(super) title: String,
    /// Its bottom left, on screen.
    pub(super) at: (f32, f32),
    pub(super) real: (usize, usize),
    pub(super) task: Option<(String, Option<TaskInfo>)>,
    pub(super) done: bool,
}

/// Every sketch at the level showing, on screen: what the canvas draws,
/// and Export Image.
#[derive(Default)]
pub(super) struct SketchPicture {
    pub(super) frames: Vec<DrawnFrame>,
    pub(super) arrows: Vec<DrawnArrow>,
    /// Notes to what they're on.
    pub(super) ties: Vec<((f32, f32), (f32, f32))>,
    pub(super) boxes: Vec<DrawnBox>,
    pub(super) notes: Vec<DrawnNote>,
    pub(super) bars: Vec<Bar>,
}

impl SketchPicture {
    pub(super) fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }
}

impl MapView {
    /// The sketches at the level showing, `screen` putting a rect of the
    /// drawing on screen.
    pub(super) fn sketch_picture(&self, screen: &dyn Fn(Rect) -> Rect) -> SketchPicture {
        let mut picture = SketchPicture::default();
        for sketch in self.sketches_here() {
            let laid = self.lay_out(sketch);
            let key = |kind: ShapeKind, id: String| {
                ShapeRef {
                    sketch: sketch.name.clone(),
                    kind,
                    id,
                }
                .key()
            };
            let selected = |key: &str| self.selected.iter().any(|k| k == key);
            for (ix, rect) in &laid.frames {
                let frame = &sketch.frames[*ix];
                let key = key(ShapeKind::Frame, frame.id.clone());
                picture.frames.push(DrawnFrame {
                    selected: selected(&key),
                    key,
                    rect: screen(*rect),
                    label: frame.label.clone(),
                });
            }
            for (ix, from, to, real) in &laid.arrows {
                let arrow = &sketch.arrows[*ix];
                let (from, to) = super::scene::edge_points(screen(*from), screen(*to));
                let key = key(ShapeKind::Arrow, ix.to_string());
                picture.arrows.push(DrawnArrow {
                    from,
                    to,
                    label: arrow.label.clone(),
                    real: *real,
                    selected: selected(&key),
                });
            }
            for (ix, on) in &laid.ties {
                if let Some((_, note)) = laid.notes.iter().find(|(at, _)| at == ix) {
                    let (a, b) = (screen(*note), screen(*on));
                    if !a.overlaps(&b) {
                        picture.ties.push(super::scene::edge_points(a, b));
                    }
                }
            }
            for (ix, rect) in &laid.boxes {
                let b = &sketch.boxes[*ix];
                let key = key(ShapeKind::Box, b.id.clone());
                let state = match self.box_match(&sketch.name, &b.id) {
                    Match::Real(place) => BoxState::Real(place),
                    Match::Unsure(n) => BoxState::Unsure(n),
                    Match::None => BoxState::Sketched,
                };
                picture.boxes.push(DrawnBox {
                    selected: selected(&key),
                    key,
                    rect: screen(*rect),
                    label: b.label.clone(),
                    purpose: b.purpose.clone(),
                    state,
                });
            }
            for (ix, rect) in &laid.notes {
                let note = &sketch.notes[*ix];
                let key = key(ShapeKind::Note, note.id.clone());
                picture.notes.push(DrawnNote {
                    selected: selected(&key),
                    key,
                    rect: screen(*rect),
                    text: note.text.clone(),
                });
            }
            if let Some(bounds) = laid.bounds().map(screen) {
                // Clear of a frame's label at the top.
                let lift = if laid
                    .frames
                    .iter()
                    .any(|(_, r)| screen(*r).y <= bounds.y + 1.)
                {
                    26.
                } else {
                    8.
                };
                picture.bars.push(Bar {
                    name: sketch.name.clone(),
                    title: sketch.title.clone(),
                    at: (bounds.x, bounds.y - lift),
                    real: self.real_boxes(&sketch.name),
                    task: sketch
                        .task
                        .clone()
                        .map(|id| (id.clone(), self.sketches.tasks.get(&id).cloned())),
                    done: self.sketch_done(&sketch.name),
                });
            }
        }
        picture
    }
}

enum Preview {
    Line(Line),
    Element(AnyElement),
}

/// The sketches' neutral ink.
pub(super) fn ink(cx: &App) -> Hsla {
    cx.theme().foreground.opacity(0.78)
}
