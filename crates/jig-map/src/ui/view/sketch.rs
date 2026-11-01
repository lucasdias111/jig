//! Sketching on the Map (`map::sketch`): the tools (pointer, box B, arrow
//! A, note N, frame F; Esc goes back to the pointer), the shapes they
//! draw, and what's done with a sketch. A sketch belongs to the level it
//! was drawn at; at other levels a box carrying one shows a mark that goes
//! there. Boxes, notes and frames are placed relative to the nearest real
//! box and move with it; an arrow or a note dragged off or dropped on a
//! real box is attached to it. Labels are edited in place, every change
//! is saved at once and ⌘Z undoes it.
//!
//! The agent is told about a sketch when its shapes are selected and ⌘K
//! is pressed, and answers beside it; Adopt turns its proposal, or a box
//! of it, into the user's sketch. Make Task has it write a planner task
//! from the sketch, which links back to the sketch file; Implement runs
//! that task. As the project is read again, boxes matched to real code
//! turn solid and their arrows snap to real calls; once every box is real
//! and the task is done, the sketch offers Archive, which takes it off the
//! canvas into the Sketches list.

use std::collections::{HashMap, HashSet};

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use super::zoom::{Camera, Transition};
use super::{Level, MapEvent, MapView};
use crate::domain::layout::Rect;
use crate::domain::lens::{Drawing, Kind as LensKind, Place, resolve, slashed};
use crate::domain::plan::{Focus, Item};
use crate::domain::sketch::{
    self, Frame, Match, Note, ShapeKind, ShapeRef, Sketch, SketchArrow, SketchBox,
};
use crate::store::cache::Folder;

mod adopting;
mod history;
mod hits;
mod labels;
mod layout;
mod mouse;
mod picking;
mod shapes;
mod telling;
pub(super) use picking::*;

/// Shapes' sizes on screen when drawn with a click, before the zoom.
pub(super) const BOX: (f32, f32) = (180., 64.);
const NOTE: (f32, f32) = (170., 76.);
const FRAME: (f32, f32) = (360., 220.);
/// A drag shorter than this, on screen, is a click.
const CLICK: f32 = 8.;
/// How near an arrow or a frame's edge, on screen, the pointer is on it.
const REACH: f32 = 7.;
/// What a new sketch is called until it's renamed.
pub const UNTITLED: &str = "Untitled sketch";

/// What a press on the canvas does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tool {
    #[default]
    Pointer,
    Box,
    Arrow,
    Note,
    Frame,
}

impl Tool {
    pub const ALL: [Tool; 5] = [
        Tool::Pointer,
        Tool::Box,
        Tool::Arrow,
        Tool::Note,
        Tool::Frame,
    ];

    /// Its key, and its name for the strip's tips.
    pub fn key(self) -> &'static str {
        match self {
            Tool::Pointer => "esc",
            Tool::Box => "b",
            Tool::Arrow => "a",
            Tool::Note => "n",
            Tool::Frame => "f",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Tool::Pointer => "Pointer",
            Tool::Box => "Box",
            Tool::Arrow => "Arrow",
            Tool::Note => "Note",
            Tool::Frame => "Frame",
        }
    }

    fn of_key(key: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|tool| tool.key() == key)
    }
}

/// A planner task as a sketch shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskInfo {
    pub title: String,
    pub done: usize,
    pub total: usize,
}

impl TaskInfo {
    pub fn finished(&self) -> bool {
        self.total > 0 && self.done == self.total
    }
}

/// What the Map keeps of the user's sketches.
#[derive(Default)]
pub struct SketchState {
    pub sketches: Vec<Sketch>,
    pub tool: Tool,
    /// The sketch new shapes go in, while it's at the level showing.
    pub active: Option<String>,
    /// What each box matches in the code, by its selection key.
    pub matches: HashMap<String, Match>,
    pub editing: Option<SketchEditing>,
    undo: Vec<Vec<Sketch>>,
    redo: Vec<Vec<Sketch>>,
    /// The Sketches list is open.
    pub list_open: bool,
    /// Ghosts of a proposal adopted, by its id and the ghost's name, with
    /// the box each became.
    pub adopted: HashMap<(usize, String), (String, String)>,
    /// The planner's tasks, by id.
    pub tasks: HashMap<String, TaskInfo>,
}

/// A label, a note's words or a sketch's title, edited in place.
pub struct SketchEditing {
    pub target: EditTarget,
    pub input: Entity<InputState>,
    _events: Subscription,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditTarget {
    Shape(ShapeRef),
    /// A sketch's title, by its name.
    Title(String),
}

/// A drag with a tool, or on a sketch's shape.
pub(super) enum SketchDrag {
    /// A tool dragging out a shape, in the window; an arrow from what it
    /// started on.
    Draw {
        tool: Tool,
        from: Point<Pixels>,
        to: Point<Pixels>,
        start: Option<End>,
    },
    /// Pressed on a shape, not moved far yet.
    Press {
        shape: ShapeRef,
        start: Point<Pixels>,
        shift: bool,
    },
    /// Moving a shape: where the pointer started, in the drawing, and where
    /// the shape was.
    Move {
        shape: ShapeRef,
        start: (f32, f32),
        from: Rect,
    },
}

/// An arrow's end, or what a note is on: a box of a sketch, or a real
/// place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum End {
    Box { sketch: String, id: String },
    Place(String),
}

/// A sketch laid out at the level showing, in the drawing.
#[derive(Clone, Debug, Default)]
pub(super) struct Laid {
    pub name: String,
    /// By their place in the sketch's lists.
    pub boxes: Vec<(usize, Rect)>,
    pub notes: Vec<(usize, Rect)>,
    pub frames: Vec<(usize, Rect)>,
    /// Each arrow's ends, and whether the code calls along it.
    pub arrows: Vec<(usize, Rect, Rect, bool)>,
    /// Each note on something, and the rect of what it's on.
    pub ties: Vec<(usize, Rect)>,
}

impl Laid {
    pub fn bounds(&self) -> Option<Rect> {
        Rect::around(
            self.boxes
                .iter()
                .chain(&self.notes)
                .chain(&self.frames)
                .map(|(_, rect)| *rect),
        )
    }
}

impl MapView {
    /// The sketches as saved now, matched against the project.
    pub(super) fn reload_sketches(&mut self) {
        self.sketches.sketches = crate::store::persist::sketches::read_all(&self.root);
        self.link_tasks();
        self.rematch();
    }

    /// The sketch named `name`.
    pub(super) fn sketch(&self, name: &str) -> Option<&Sketch> {
        self.sketches.sketches.iter().find(|s| s.name == name)
    }

    /// Sketches drawn at the level showing, not archived.
    pub(super) fn sketches_here(&self) -> impl Iterator<Item = &Sketch> {
        let here = self.view_key();
        self.sketches
            .sketches
            .iter()
            .filter(move |s| !s.archived && s.level == here)
    }

    pub fn tool(&self) -> Tool {
        self.sketches.tool
    }

    /// Pick `tool` (B, A, N, F; the pointer with Esc).
    pub fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.sketches.tool = tool;
        if tool != Tool::Pointer {
            self.drag = None;
        }
        cx.notify();
    }

    /// The tool a key picks, if it picks one.
    pub(super) fn tool_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        match Tool::of_key(key) {
            Some(tool) if tool != Tool::Pointer && self.project.is_some() => {
                self.set_tool(tool, cx);
                true
            }
            _ => false,
        }
    }

    /// The level a sketch's `level` names, as the project has it now.
    pub(super) fn level_named(&self, key: &str) -> Option<Level> {
        match key {
            "site" => Some(Level::Site),
            "domain" => Some(Level::Domain),
            _ => {
                let place = key.strip_prefix("plan:")?;
                let function = self.project.as_ref().and_then(|project| {
                    let id = resolve(project, None, &Folder(&self.root), place)
                        .ok()?
                        .function_in(project)?;
                    Some(project.symbol(id))
                });
                Some(Level::Plan(match function {
                    Some(symbol) => Focus::Function(symbol),
                    None => Focus::Module(place.to_string()),
                }))
            }
        }
    }
}

/// Whether a sketch is kept: it holds something, or a task was made of it.
fn worth_keeping(sketch: &Sketch) -> bool {
    !sketch.is_empty() || sketch.task.is_some()
}

/// How far drawing point `at` is from `rect` (0 inside it).
fn distance(rect: Rect, (x, y): (f32, f32)) -> f32 {
    let dx = (rect.x - x).max(0.).max(x - rect.right());
    let dy = (rect.y - y).max(0.).max(y - rect.bottom());
    dx.hypot(dy)
}

/// Positions are kept to whole points, so the files read well.
fn round(value: f32) -> f32 {
    value.round()
}

#[cfg(any(test, feature = "testing"))]
impl MapView {
    pub fn saved_sketches(&self) -> &[Sketch] {
        &self.sketches.sketches
    }

    /// Where shape `key` is drawn in the window, its middle.
    pub fn shape_point(&self, key: &str) -> Point<Pixels> {
        let shape = ShapeRef::of_key(key).expect("a shape's key");
        let rect = self
            .shape_rect(&shape)
            .unwrap_or_else(|| panic!("{key} isn't drawn"));
        self.bounds.get().origin + self.to_canvas(rect.centre())
    }

    /// Where shape `key` is in the drawing.
    pub fn shape_at(&self, key: &str) -> Option<Rect> {
        self.shape_rect(&ShapeRef::of_key(key)?)
    }

    /// A point on the canvas, in the window, well clear of every box and
    /// shape, nearest its middle.
    pub fn empty_point(&self) -> Point<Pixels> {
        let canvas = self.bounds.get();
        let (w, h) = (f32::from(canvas.size.width), f32::from(canvas.size.height));
        let mut spots: Vec<(f32, f32)> = Vec::new();
        for gx in 2..(w / 20.) as i32 - 2 {
            for gy in 2..(h / 20.) as i32 - 2 {
                spots.push((gx as f32 * 20., gy as f32 * 20.));
            }
        }
        spots.sort_by(|a, b| {
            let d = |p: &(f32, f32)| (p.0 - w / 2.).hypot(p.1 - h / 2.);
            d(a).total_cmp(&d(b))
        });
        let clear = |at: Point<Pixels>| {
            [
                (0., 0.),
                (-110., -50.),
                (110., 50.),
                (-110., 50.),
                (110., -50.),
            ]
            .iter()
            .all(|(dx, dy)| {
                let p = at + point(px(*dx), px(*dy));
                self.real_place_at(p).is_none() && self.sketch_at(p).is_none()
            })
        };
        spots
            .into_iter()
            .map(|(x, y)| canvas.origin + point(px(x), px(y)))
            .find(|at| clear(*at))
            .expect("somewhere clear")
    }

    pub fn match_of(&self, sketch: &str, id: &str) -> Match {
        self.box_match(sketch, id)
    }

    /// The marks of other levels' sketches drawn here: the box, and the
    /// sketches.
    pub fn marks(&self) -> Vec<(String, Vec<String>)> {
        self.sketch_marks()
            .into_iter()
            .map(|(landing, names)| (landing.key, names))
            .collect()
    }

    /// The sketches drawn at the level showing, by name.
    pub fn sketches_showing(&self) -> Vec<String> {
        self.sketches_here().map(|s| s.name.clone()).collect()
    }
}
