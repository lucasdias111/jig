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

use super::super::layout::Rect;
use super::super::lens::{Drawing, Kind as LensKind, Place, resolve, slashed};
use super::super::plan::{Focus, Item};
use super::super::sketch::{
    self, Frame, Match, Note, ShapeKind, ShapeRef, Sketch, SketchArrow, SketchBox,
};
use super::zoom::{Camera, Transition};
use super::{Level, MapEvent, MapView};

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
        self.sketches.sketches = sketch::read_all(&self.root);
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
                    let id = resolve(project, None, place).ok()?.function_in(project)?;
                    Some(project.symbol(id))
                });
                Some(Level::Plan(match function {
                    Some(symbol) => Focus::Function(symbol),
                    None => Focus::Module(place.to_string()),
                }))
            }
        }
    }

    // Laying sketches out.

    /// The rect of real place `name` that shapes are placed by: at Site its
    /// own box, wherever it is, so shapes don't move as boxes open; else
    /// the box it lands on.
    fn anchor_rect(&self, name: &str) -> Option<Rect> {
        if name.is_empty() {
            return None;
        }
        match self.level {
            Level::Site => {
                let place = self.place(name)?;
                let ix = self.site_box_of_place(&place)?;
                Some(self.site_nodes()[ix].rect)
            }
            _ => self.landing(name).map(|landing| landing.rect),
        }
    }

    /// Where a shape placed `at` from `near`'s box is, `size` big.
    fn placed(&self, near: &str, at: [f32; 2], size: [f32; 2]) -> Rect {
        let (x, y) = match self.anchor_rect(near) {
            Some(anchor) => (anchor.x + at[0], anchor.y + at[1]),
            None => (at[0], at[1]),
        };
        Rect {
            x,
            y,
            width: size[0],
            height: size[1],
        }
    }

    /// Sketch `sketch` laid out at the level showing.
    pub(super) fn lay_out(&self, sketch: &Sketch) -> Laid {
        let boxes: Vec<(usize, Rect)> = sketch
            .boxes
            .iter()
            .enumerate()
            .map(|(ix, b)| (ix, self.placed(&b.near, b.at, b.size)))
            .collect();
        let box_rect = |id: &str| {
            sketch
                .boxes
                .iter()
                .position(|b| b.id == id)
                .map(|ix| boxes[ix].1)
        };
        // An end: one of its boxes, else a real box as far as it's open.
        let end_rect =
            |end: &str| box_rect(end).or_else(|| self.landing(end).map(|landing| landing.rect));
        let mut notes = Vec::new();
        let mut ties = Vec::new();
        for (ix, note) in sketch.notes.iter().enumerate() {
            let rect = match (!note.on.is_empty()).then(|| end_rect(&note.on)).flatten() {
                Some(on) => {
                    ties.push((ix, on));
                    Rect {
                        x: on.x + note.at[0],
                        y: on.y + note.at[1],
                        width: note.size[0],
                        height: note.size[1],
                    }
                }
                None => self.placed(&note.near, note.at, note.size),
            };
            notes.push((ix, rect));
        }
        let frames = sketch
            .frames
            .iter()
            .enumerate()
            .map(|(ix, f)| (ix, self.placed(&f.near, f.at, f.size)))
            .collect();
        let arrows = sketch
            .arrows
            .iter()
            .enumerate()
            .filter_map(|(ix, arrow)| {
                let from = end_rect(&arrow.from)?;
                let to = end_rect(&arrow.to)?;
                Some((ix, from, to, self.arrow_real(sketch, arrow)))
            })
            .collect();
        Laid {
            name: sketch.name.clone(),
            boxes,
            notes,
            frames,
            arrows,
            ties,
        }
    }

    /// Where the sketches at the level showing are, in the drawing.
    pub(super) fn sketches_bounds(&self) -> Option<Rect> {
        Rect::around(self.laid_sketches().iter().filter_map(Laid::bounds))
    }

    /// Every sketch at the level showing, laid out.
    pub(super) fn laid_sketches(&self) -> Vec<Laid> {
        self.sketches_here().map(|s| self.lay_out(s)).collect()
    }

    // Matching to code.

    /// What box `id` of sketch `name` matches in the code.
    pub(super) fn box_match(&self, name: &str, id: &str) -> Match {
        let key = ShapeRef {
            sketch: name.into(),
            kind: ShapeKind::Box,
            id: id.into(),
        }
        .key();
        self.sketches
            .matches
            .get(&key)
            .cloned()
            .unwrap_or(Match::None)
    }

    /// Every box matched against the project as read now.
    pub(super) fn rematch(&mut self) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let mut matches = HashMap::new();
        for sketch in &self.sketches.sketches {
            for b in &sketch.boxes {
                let near = (!b.near.is_empty()).then_some(b.near.as_str());
                let key = ShapeRef {
                    sketch: sketch.name.clone(),
                    kind: ShapeKind::Box,
                    id: b.id.clone(),
                }
                .key();
                matches.insert(key, sketch::match_box(&project, &b.label, near));
            }
        }
        self.sketches.matches = matches;
    }

    /// The real place an arrow's end is: a matched box's code, else the
    /// place itself.
    fn real_end(&self, sketch: &Sketch, end: &str) -> Option<Place> {
        if sketch.is_box(end) {
            let place = self.box_match(&sketch.name, end).place()?.to_string();
            return self.place(&place);
        }
        self.place(end)
    }

    /// Whether the code calls along `arrow`, one way or the other, between
    /// two real ends.
    pub(super) fn arrow_real(&self, sketch: &Sketch, arrow: &SketchArrow) -> bool {
        let Some(project) = self.project.as_deref() else {
            return false;
        };
        let (Some(from), Some(to)) = (
            self.real_end(sketch, &arrow.from),
            self.real_end(sketch, &arrow.to),
        ) else {
            return false;
        };
        let (from, to) = (
            sketch::functions_in(project, &from),
            sketch::functions_in(project, &to),
        );
        sketch::calls_between(project, &from, &to) || sketch::calls_between(project, &to, &from)
    }

    /// How many of a sketch's boxes are real code now.
    pub fn real_boxes(&self, name: &str) -> (usize, usize) {
        let Some(sketch) = self.sketch(name) else {
            return (0, 0);
        };
        let real = sketch
            .boxes
            .iter()
            .filter(|b| matches!(self.box_match(name, &b.id), Match::Real(_)))
            .count();
        (real, sketch.boxes.len())
    }

    /// Whether sketch `name` is done: every box real, and its task (if it
    /// has one) done.
    pub fn sketch_done(&self, name: &str) -> bool {
        let Some(sketch) = self.sketch(name) else {
            return false;
        };
        let (real, total) = self.real_boxes(name);
        let task_done = sketch
            .task
            .as_ref()
            .is_none_or(|id| self.sketches.tasks.get(id).is_some_and(TaskInfo::finished));
        total > 0 && real == total && task_done
    }

    // Tasks.

    /// The planner's tasks read again: a task whose description links a
    /// sketch file becomes that sketch's task.
    pub fn link_tasks(&mut self) {
        let tasks = jig_ai::tasks::list(&self.root).unwrap_or_default();
        self.sketches.tasks = tasks
            .iter()
            .map(|task| {
                (
                    task.id.clone(),
                    TaskInfo {
                        title: task.title.clone(),
                        done: task.done(),
                        total: task.todos.len(),
                    },
                )
            })
            .collect();
        let mut linked = Vec::new();
        for task in &tasks {
            for name in sketch::linked_in(&task.description) {
                if let Some(sketch) = self.sketch(&name)
                    && sketch.task.as_deref() != Some(task.id.as_str())
                    && sketch
                        .task
                        .as_ref()
                        .is_none_or(|was| !self.sketches.tasks.contains_key(was))
                {
                    linked.push((name, task.id.clone()));
                }
            }
        }
        for (name, id) in linked {
            if let Some(sketch) = self.sketches.sketches.iter_mut().find(|s| s.name == name) {
                sketch.task = Some(id);
                let _ = sketch::save(&self.root, sketch);
            }
        }
    }

    // Editing, saved at once and undone with ⌘Z.

    /// Change the sketches: remembered for ⌘Z, saved, matched again.
    pub(super) fn edit_sketches(
        &mut self,
        edit: impl FnOnce(&mut Vec<Sketch>),
        cx: &mut Context<Self>,
    ) {
        let mut before = self.sketches.sketches.clone();
        // A sketch is saved with its first shape, and goes with its last.
        before.retain(worth_keeping);
        edit(&mut self.sketches.sketches);
        self.sketches.sketches.retain(worth_keeping);
        if self.sketches.sketches == before {
            return;
        }
        self.persist(&before);
        self.sketches.undo.push(before);
        self.sketches.redo.clear();
        self.rematch();
        cx.notify();
    }

    /// Write the sketches that changed since `before`, and remove those
    /// gone.
    fn persist(&mut self, before: &[Sketch]) {
        for sketch in &self.sketches.sketches {
            if !worth_keeping(sketch)
                || before
                    .iter()
                    .any(|was| was == sketch && was.name == sketch.name)
            {
                continue;
            }
            if let Err(error) = sketch::save(&self.root, sketch) {
                self.notice = Some((format!("{error:#}"), std::time::Instant::now()));
            }
        }
        for was in before {
            if !self
                .sketches
                .sketches
                .iter()
                .any(|s| s.name == was.name && worth_keeping(s))
            {
                let _ = sketch::remove(&self.root, &was.name);
            }
        }
    }

    /// ⌘Z on the sketches: the last change undone. Whether there was one.
    pub fn undo_sketches(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(was) = self.sketches.undo.pop() else {
            return false;
        };
        let now = std::mem::replace(&mut self.sketches.sketches, was);
        self.persist(&now);
        self.sketches.redo.push(now);
        self.after_undo(cx);
        true
    }

    /// ⇧⌘Z: the change undone, again.
    pub fn redo_sketches(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(again) = self.sketches.redo.pop() else {
            return false;
        };
        let now = std::mem::replace(&mut self.sketches.sketches, again);
        self.persist(&now);
        self.sketches.undo.push(now);
        self.after_undo(cx);
        true
    }

    fn after_undo(&mut self, cx: &mut Context<Self>) {
        self.sketches.editing = None;
        let shapes = self.all_shape_keys();
        self.selected
            .retain(|key| !key.starts_with(sketch::PREFIX) || shapes.contains(key));
        self.rematch();
        cx.notify();
    }

    fn all_shape_keys(&self) -> HashSet<String> {
        self.sketches
            .sketches
            .iter()
            .flat_map(|s| shapes_of(s).into_iter().map(|shape| shape.key()))
            .collect()
    }

    /// The sketch new shapes go in: `preferred`, else the one being drawn
    /// at this level, else a new one.
    fn sketch_for_new_shape(&mut self, preferred: Option<&str>) -> String {
        let here = self.view_key();
        let usable = |name: &str| {
            self.sketch(name)
                .is_some_and(|s| !s.archived && s.level == here)
        };
        if let Some(name) = preferred.filter(|name| usable(name)) {
            return name.to_string();
        }
        if let Some(name) = self.sketches.active.clone().filter(|name| usable(name)) {
            return name;
        }
        let taken: Vec<String> = self
            .sketches
            .sketches
            .iter()
            .map(|s| s.name.clone())
            .collect();
        let sketch = Sketch::new(UNTITLED, &here, &taken);
        let name = sketch.name.clone();
        // Saved with its first shape.
        self.sketches.sketches.push(sketch);
        name
    }

    /// The real box nearest drawing point `at`, by place name: the
    /// innermost box holding it, else the closest.
    fn nearest_real(&self, at: (f32, f32)) -> Option<(String, Rect)> {
        let candidates = self.real_boxes_here();
        let holding = candidates
            .iter()
            .filter(|(_, rect)| rect.contains(at))
            .min_by(|a, b| (a.1.width * a.1.height).total_cmp(&(b.1.width * b.1.height)));
        holding
            .or_else(|| {
                candidates
                    .iter()
                    .min_by(|a, b| distance(a.1, at).total_cmp(&distance(b.1, at)))
            })
            .cloned()
    }

    /// The level's real boxes, by place name, where they are.
    fn real_boxes_here(&self) -> Vec<(String, Rect)> {
        let mut boxes: Vec<(String, Rect)> = match &self.level {
            Level::Plan(_) => self
                .plan
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(ix, _)| Some((self.plan_place(ix)?, self.placed.nodes[ix])))
                .collect(),
            Level::Site => self
                .site_nodes()
                .iter()
                .skip(1)
                .map(|node| {
                    let name = if node.place.is_empty() {
                        node.key.clone()
                    } else {
                        node.place.clone()
                    };
                    (name, node.rect)
                })
                .collect(),
            Level::Domain => self
                .domain_level()
                .map(|level| {
                    level
                        .showing()
                        .map(|(_, region)| (region.key.clone(), region.rect))
                        .collect()
                })
                .unwrap_or_default(),
        };
        boxes.retain(|(name, _)| self.place(name).is_some());
        boxes
    }

    /// Plan box `ix` as a place: its function, or its file.
    fn plan_place(&self, ix: usize) -> Option<String> {
        let project = self.project.as_ref()?;
        match &self.plan.nodes.get(ix)?.item {
            Item::Function(id) => {
                let function = &project.functions[*id];
                Some(format!(
                    "{}#{}",
                    slashed(&project.files[function.file].path),
                    function.title()
                ))
            }
            Item::File { path, .. } => Some(path.clone()),
            _ => None,
        }
    }

    /// The real box under `position` in the window, as a place.
    fn real_place_at(&self, position: Point<Pixels>) -> Option<String> {
        let name = match self.level {
            Level::Plan(_) => self.plan_place(self.plan_box_at(position)?)?,
            Level::Site => {
                let ix = self.site_box_at(position).filter(|&ix| ix != 0)?;
                let node = &self.site_nodes()[ix];
                if node.place.is_empty() {
                    node.key.clone()
                } else {
                    node.place.clone()
                }
            }
            Level::Domain => {
                let ix = self.region_at(position)?;
                self.domain_level()?.regions.get(ix)?.key.clone()
            }
        };
        self.place(&name).is_some().then_some(name)
    }

    /// Where a shape at drawing rect `rect` is placed from: the nearest
    /// real box and where it is from it, else where it is.
    fn anchor(&self, rect: Rect) -> (String, [f32; 2]) {
        match self.nearest_real(rect.centre()) {
            Some((name, anchor)) => (name, [round(rect.x - anchor.x), round(rect.y - anchor.y)]),
            None => (String::new(), [round(rect.x), round(rect.y)]),
        }
    }

    /// A drawing size `screen` points big on screen now.
    fn drawing_size(&self, (width, height): (f32, f32)) -> [f32; 2] {
        let zoom = self.zoom.max(0.0001);
        [round(width / zoom), round(height / zoom)]
    }

    // Hitting shapes.

    /// The sketch shape under `position`: a note, a box, an arrow, then a
    /// frame's label or edge.
    pub(super) fn sketch_at(&self, position: Point<Pixels>) -> Option<ShapeRef> {
        let at = self.to_drawing(position);
        let reach = REACH / self.zoom.max(0.0001);
        let laid = self.laid_sketches();
        let shape = |laid: &Laid, kind: ShapeKind, id: String| ShapeRef {
            sketch: laid.name.clone(),
            kind,
            id,
        };
        for laid in laid.iter().rev() {
            let Some(sketch) = self.sketch(&laid.name) else {
                continue;
            };
            if let Some((ix, _)) = laid.notes.iter().rev().find(|(_, r)| r.contains(at)) {
                return Some(shape(laid, ShapeKind::Note, sketch.notes[*ix].id.clone()));
            }
            if let Some((ix, _)) = laid.boxes.iter().rev().find(|(_, r)| r.contains(at)) {
                return Some(shape(laid, ShapeKind::Box, sketch.boxes[*ix].id.clone()));
            }
        }
        for laid in laid.iter().rev() {
            let near_arrow = laid.arrows.iter().find(|(_, from, to, _)| {
                let (a, b) = super::scene::edge_points(*from, *to);
                super::input::distance_to_bow(a, b, at) < reach
            });
            if let Some((ix, ..)) = near_arrow {
                return Some(shape(laid, ShapeKind::Arrow, ix.to_string()));
            }
            let Some(sketch) = self.sketch(&laid.name) else {
                continue;
            };
            let label = 24. / self.zoom.max(0.0001);
            let on_frame = laid.frames.iter().rev().find(|(_, r)| {
                let title = Rect {
                    x: r.x,
                    y: r.y - label,
                    width: r.width.min(240. / self.zoom.max(0.0001)),
                    height: label,
                };
                let outer = grow(*r, reach);
                let inner = grow(*r, -reach);
                title.contains(at) || (outer.contains(at) && !inner.contains(at))
            });
            if let Some((ix, _)) = on_frame {
                return Some(shape(laid, ShapeKind::Frame, sketch.frames[*ix].id.clone()));
            }
        }
        None
    }

    /// The rect shape `shape` is drawn at, in the drawing.
    pub(super) fn shape_rect(&self, shape: &ShapeRef) -> Option<Rect> {
        let sketch = self.sketch(&shape.sketch)?;
        let laid = self.lay_out(sketch);
        let find = |list: &[(usize, Rect)], ids: Vec<&str>| {
            list.iter()
                .find(|(ix, _)| ids.get(*ix) == Some(&shape.id.as_str()))
                .map(|(_, r)| *r)
        };
        match shape.kind {
            ShapeKind::Box => find(
                &laid.boxes,
                sketch.boxes.iter().map(|b| b.id.as_str()).collect(),
            ),
            ShapeKind::Note => find(
                &laid.notes,
                sketch.notes.iter().map(|n| n.id.as_str()).collect(),
            ),
            ShapeKind::Frame => find(
                &laid.frames,
                sketch.frames.iter().map(|f| f.id.as_str()).collect(),
            ),
            ShapeKind::Arrow => {
                let ix: usize = shape.id.parse().ok()?;
                let (_, from, to, _) = laid.arrows.iter().find(|(at, ..)| *at == ix)?;
                Some(from.union(to))
            }
        }
    }

    // The mouse.

    /// A press on the canvas with a tool, or on a sketch's shape: taken
    /// here, or left to the Map.
    pub(super) fn sketch_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let position = event.position;
        if self.sketches.editing.is_some() {
            self.finish_sketch_edit(true, cx);
        }
        let tool = self.sketches.tool;
        if tool != Tool::Pointer {
            let start = match tool {
                Tool::Arrow => self.end_at(position),
                _ => None,
            };
            self.drag = Some(super::input::Drag::Sketch(SketchDrag::Draw {
                tool,
                from: position,
                to: position,
                start,
            }));
            return true;
        }
        let Some(shape) = self.sketch_at(position) else {
            return false;
        };
        self.sketches.active = Some(shape.sketch.clone());
        if event.click_count >= 2 {
            self.selected = vec![shape.key()];
            self.begin_sketch_edit(EditTarget::Shape(shape), false, window, cx);
            return true;
        }
        self.drag = Some(super::input::Drag::Sketch(SketchDrag::Press {
            shape,
            start: position,
            shift: event.modifiers.shift,
        }));
        true
    }

    /// What an arrow would start or end on at `position`: a sketch's box,
    /// else a real box.
    fn end_at(&self, position: Point<Pixels>) -> Option<End> {
        if let Some(shape) = self.sketch_at(position)
            && shape.kind == ShapeKind::Box
        {
            return Some(End::Box {
                sketch: shape.sketch,
                id: shape.id,
            });
        }
        self.real_place_at(position).map(End::Place)
    }

    /// The pointer moved with a sketch drag under way: the drag carried on.
    pub(super) fn sketch_mouse_move(
        &mut self,
        drag: SketchDrag,
        position: Point<Pixels>,
    ) -> SketchDrag {
        match drag {
            SketchDrag::Draw {
                tool, from, start, ..
            } => SketchDrag::Draw {
                tool,
                from,
                to: position,
                start,
            },
            SketchDrag::Press { shape, start, .. }
                if super::input::far(position, start) && shape.kind != ShapeKind::Arrow =>
            {
                let from = self.shape_rect(&shape).unwrap_or_default();
                // Remembered for ⌘Z before it moves.
                self.sketches.undo.push(self.sketches.sketches.clone());
                self.sketches.redo.clear();
                let start = self.to_drawing(start);
                if !self.selected.contains(&shape.key()) {
                    self.selected = vec![shape.key()];
                }
                let drag = SketchDrag::Move { shape, start, from };
                self.move_shape(&drag, position);
                drag
            }
            SketchDrag::Move { .. } => {
                self.move_shape(&drag, position);
                drag
            }
            other => other,
        }
    }

    /// A shape being moved, where the pointer is: placed where it is in
    /// the drawing until it's let go.
    fn move_shape(&mut self, drag: &SketchDrag, position: Point<Pixels>) {
        let SketchDrag::Move { shape, start, from } = drag else {
            return;
        };
        let (x, y) = self.to_drawing(position);
        let to = Rect {
            x: from.x + x - start.0,
            y: from.y + y - start.1,
            ..*from
        };
        let on = self.note_target(shape);
        let Some(sketch) = self
            .sketches
            .sketches
            .iter_mut()
            .find(|s| s.name == shape.sketch)
        else {
            return;
        };
        let at = [to.x, to.y];
        match shape.kind {
            ShapeKind::Box => {
                if let Some(b) = sketch.boxes.iter_mut().find(|b| b.id == shape.id) {
                    b.near.clear();
                    b.at = at;
                }
            }
            ShapeKind::Note => {
                if let Some(n) = sketch.notes.iter_mut().find(|n| n.id == shape.id) {
                    match on {
                        // A note on something stays on it.
                        Some(on) => n.at = [to.x - on.x, to.y - on.y],
                        None => {
                            n.near.clear();
                            n.at = at;
                        }
                    }
                }
            }
            ShapeKind::Frame => {
                if let Some(f) = sketch.frames.iter_mut().find(|f| f.id == shape.id) {
                    f.near.clear();
                    f.at = at;
                }
            }
            ShapeKind::Arrow => {}
        }
    }

    /// The rect of what note `shape` is on, if it's on something.
    fn note_target(&self, shape: &ShapeRef) -> Option<Rect> {
        if shape.kind != ShapeKind::Note {
            return None;
        }
        let sketch = self.sketch(&shape.sketch)?;
        let note = sketch.notes.iter().find(|n| n.id == shape.id)?;
        let laid = self.lay_out(sketch);
        laid.ties
            .iter()
            .find(|(ix, _)| sketch.notes[*ix].id == note.id)
            .map(|(_, on)| *on)
    }

    /// The press let go: a shape drawn, selected or put down.
    pub(super) fn sketch_mouse_up(
        &mut self,
        drag: SketchDrag,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match drag {
            SketchDrag::Draw {
                tool,
                from,
                to,
                start,
            } => self.draw_shape(tool, from, to, start, window, cx),
            SketchDrag::Press { shape, shift, .. } => {
                let key = shape.key();
                if shift {
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
            SketchDrag::Move { shape, .. } => self.put_down(&shape, cx),
        }
        cx.notify();
    }

    /// A moved shape put down: placed from the nearest real box there, and
    /// saved.
    fn put_down(&mut self, shape: &ShapeRef, cx: &mut Context<Self>) {
        let Some(rect) = self.shape_rect(shape) else {
            return;
        };
        let attached = self.note_target(shape).is_some();
        let (near, at) = self.anchor(rect);
        let before = self.sketches.undo.last().cloned().unwrap_or_default();
        if let Some(sketch) = self
            .sketches
            .sketches
            .iter_mut()
            .find(|s| s.name == shape.sketch)
        {
            match shape.kind {
                ShapeKind::Box => {
                    if let Some(b) = sketch.boxes.iter_mut().find(|b| b.id == shape.id) {
                        (b.near, b.at) = (near, at);
                    }
                }
                ShapeKind::Note if !attached => {
                    if let Some(n) = sketch.notes.iter_mut().find(|n| n.id == shape.id) {
                        (n.near, n.at) = (near, at);
                    }
                }
                ShapeKind::Note => {
                    if let Some(n) = sketch.notes.iter_mut().find(|n| n.id == shape.id) {
                        n.at = [round(n.at[0]), round(n.at[1])];
                    }
                }
                ShapeKind::Frame => {
                    if let Some(f) = sketch.frames.iter_mut().find(|f| f.id == shape.id) {
                        (f.near, f.at) = (near, at);
                    }
                }
                ShapeKind::Arrow => {}
            }
        }
        self.persist(&before);
        self.rematch();
        cx.notify();
    }

    /// A tool's drag let go: its shape made, and its label edited.
    fn draw_shape(
        &mut self,
        tool: Tool,
        from: Point<Pixels>,
        to: Point<Pixels>,
        start: Option<End>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sketches.tool = Tool::Pointer;
        let click = !super::input::far_by(from, to, CLICK);
        let (a, b) = (self.to_drawing(from), self.to_drawing(to));
        let dragged = Rect {
            x: a.0.min(b.0),
            y: a.1.min(b.1),
            width: (a.0 - b.0).abs(),
            height: (a.1 - b.1).abs(),
        };
        let made = match tool {
            Tool::Pointer => None,
            Tool::Box => {
                let rect = if click { self.centred(b, BOX) } else { dragged };
                Some(self.add_box(None, "Box", rect, cx))
            }
            Tool::Frame => {
                let rect = if click {
                    self.centred(b, FRAME)
                } else {
                    dragged
                };
                Some(self.add_frame(rect, cx))
            }
            Tool::Note => Some(self.add_note(to, cx)),
            Tool::Arrow => self.add_arrow(start, to, cx),
        };
        if let Some(shape) = made {
            self.selected = vec![shape.key()];
            self.begin_sketch_edit(EditTarget::Shape(shape), true, window, cx);
        }
    }

    /// A rect `size` big on screen, centred on drawing point `at`.
    fn centred(&self, at: (f32, f32), size: (f32, f32)) -> Rect {
        let [width, height] = self.drawing_size(size);
        Rect {
            x: at.0 - width / 2.,
            y: at.1 - height / 2.,
            width,
            height,
        }
    }

    /// A new box labelled `label` at drawing rect `rect`, in `sketch` or
    /// the one being drawn.
    fn add_box(
        &mut self,
        sketch: Option<&str>,
        label: &str,
        rect: Rect,
        cx: &mut Context<Self>,
    ) -> ShapeRef {
        let name = self.sketch_for_new_shape(sketch);
        let (near, at) = self.anchor(rect);
        let size = [round(rect.width), round(rect.height)];
        let mut id = String::new();
        self.edit_sketches(
            |sketches| {
                if let Some(sketch) = sketches.iter_mut().find(|s| s.name == name) {
                    id = sketch.new_id(label, "box");
                    sketch.boxes.push(SketchBox {
                        id: id.clone(),
                        label: label.to_string(),
                        purpose: String::new(),
                        near,
                        at,
                        size,
                    });
                }
            },
            cx,
        );
        self.sketches.active = Some(name.clone());
        ShapeRef {
            sketch: name,
            kind: ShapeKind::Box,
            id,
        }
    }

    fn add_frame(&mut self, rect: Rect, cx: &mut Context<Self>) -> ShapeRef {
        let name = self.sketch_for_new_shape(None);
        let (near, at) = self.anchor(rect);
        let size = [round(rect.width), round(rect.height)];
        let mut id = String::new();
        self.edit_sketches(
            |sketches| {
                if let Some(sketch) = sketches.iter_mut().find(|s| s.name == name) {
                    id = sketch.new_id("frame", "frame");
                    sketch.frames.push(Frame {
                        id: id.clone(),
                        label: "Frame".into(),
                        near,
                        at,
                        size,
                    });
                }
            },
            cx,
        );
        self.sketches.active = Some(name.clone());
        ShapeRef {
            sketch: name,
            kind: ShapeKind::Frame,
            id,
        }
    }

    /// A note where the pointer let go: on the box there (a sketch's or a
    /// real one), beside it, else floating.
    fn add_note(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> ShapeRef {
        let on = self.end_at(position);
        let preferred = match &on {
            Some(End::Box { sketch, .. }) => Some(sketch.clone()),
            _ => None,
        };
        let name = self.sketch_for_new_shape(preferred.as_deref());
        let size = self.drawing_size(NOTE);
        let gap = 14. / self.zoom.max(0.0001);
        let (on, near, at) = match on {
            Some(End::Box { sketch, id }) if sketch == name => {
                let width = self
                    .shape_rect(&ShapeRef {
                        sketch,
                        kind: ShapeKind::Box,
                        id: id.clone(),
                    })
                    .map_or(0., |r| r.width);
                (id, String::new(), [round(width + gap), 0.])
            }
            Some(End::Place(place)) => {
                let width = self.landing(&place).map_or(0., |l| l.rect.width);
                (place, String::new(), [round(width + gap), 0.])
            }
            _ => {
                let rect = self.centred(self.to_drawing(position), NOTE);
                let (near, at) = self.anchor(rect);
                (String::new(), near, at)
            }
        };
        let mut id = String::new();
        self.edit_sketches(
            |sketches| {
                if let Some(sketch) = sketches.iter_mut().find(|s| s.name == name) {
                    id = sketch.new_id("note", "note");
                    sketch.notes.push(Note {
                        id: id.clone(),
                        text: String::new(),
                        on,
                        near,
                        at,
                        size,
                    });
                }
            },
            cx,
        );
        self.sketches.active = Some(name.clone());
        ShapeRef {
            sketch: name,
            kind: ShapeKind::Note,
            id,
        }
    }

    /// An arrow from `start` to what's under `position`; an end on nothing
    /// is a new box there. Both ends nothing, or boxes of two sketches,
    /// makes no arrow.
    fn add_arrow(
        &mut self,
        start: Option<End>,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> Option<ShapeRef> {
        let end = self.end_at(position);
        let sketch_of = |end: &Option<End>| match end {
            Some(End::Box { sketch, .. }) => Some(sketch.clone()),
            _ => None,
        };
        let (a, b) = (sketch_of(&start), sketch_of(&end));
        if let (Some(a), Some(b)) = (&a, &b)
            && a != b
        {
            self.say("An arrow joins boxes of one sketch.", cx);
            return None;
        }
        let start = start?;
        if end == Some(start.clone()) {
            return None;
        }
        let name = self.sketch_for_new_shape(a.or(b).as_deref());
        let id_of = |end: End| match end {
            End::Box { id, .. } => id,
            End::Place(place) => place,
        };
        let (to, made) = match end {
            Some(end) => (id_of(end), None),
            None => {
                let rect = self.centred(self.to_drawing(position), BOX);
                let made = self.add_box(Some(&name), "Box", rect, cx);
                (made.id.clone(), Some(made))
            }
        };
        let from = id_of(start);
        let mut ix = 0;
        self.edit_sketches(
            |sketches| {
                if let Some(sketch) = sketches.iter_mut().find(|s| s.name == name) {
                    ix = sketch.arrows.len();
                    sketch.arrows.push(SketchArrow {
                        from,
                        to,
                        label: String::new(),
                    });
                }
            },
            cx,
        );
        self.sketches.active = Some(name.clone());
        // A new box at its end gets its label first; else the arrow's.
        Some(made.unwrap_or(ShapeRef {
            sketch: name,
            kind: ShapeKind::Arrow,
            id: ix.to_string(),
        }))
    }

    /// Delete or ⌫: the selected shapes taken out, a box with its arrows
    /// and notes. Whether any were.
    pub(super) fn delete_selected_shapes(&mut self, cx: &mut Context<Self>) -> bool {
        let shapes: Vec<ShapeRef> = self
            .selected
            .iter()
            .filter_map(|key| ShapeRef::of_key(key))
            .collect();
        if shapes.is_empty() {
            return false;
        }
        self.edit_sketches(
            |sketches| {
                for sketch in sketches.iter_mut() {
                    let name = sketch.name.clone();
                    let mine: Vec<&ShapeRef> = shapes.iter().filter(|s| s.sketch == name).collect();
                    // Arrows first: taking out a box renumbers them.
                    let mut arrows: Vec<usize> = mine
                        .iter()
                        .filter(|s| s.kind == ShapeKind::Arrow)
                        .filter_map(|s| s.id.parse().ok())
                        .collect();
                    arrows.sort_unstable();
                    for ix in arrows.into_iter().rev() {
                        if ix < sketch.arrows.len() {
                            sketch.arrows.remove(ix);
                        }
                    }
                    for shape in mine {
                        match shape.kind {
                            ShapeKind::Box => sketch.remove_box(&shape.id),
                            ShapeKind::Note => sketch.notes.retain(|n| n.id != shape.id),
                            ShapeKind::Frame => sketch.frames.retain(|f| f.id != shape.id),
                            ShapeKind::Arrow => {}
                        }
                    }
                }
            },
            cx,
        );
        self.selected.retain(|key| !key.starts_with(sketch::PREFIX));
        true
    }

    // Editing in place.

    /// Edit a shape's label or a sketch's title in place; a shape just
    /// drawn starts empty, keeping its label if nothing's typed.
    pub fn begin_sketch_edit(
        &mut self,
        target: EditTarget,
        fresh: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(value) = self.edit_value(&target) else {
            return;
        };
        let placeholder = match &target {
            EditTarget::Title(_) => "Title",
            EditTarget::Shape(shape) => match shape.kind {
                ShapeKind::Box => "What should be here?",
                ShapeKind::Note => "A note",
                ShapeKind::Frame => "Frame",
                ShapeKind::Arrow => "What flows along it",
            },
        };
        let value = if fresh { String::new() } else { value };
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(value)
        });
        let events = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::PressEnter { .. } => {
                this.finish_sketch_edit(true, cx);
                this.focus.focus(window, cx);
            }
            InputEvent::Blur => this.finish_sketch_edit(true, cx),
            _ => {}
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.sketches.editing = Some(SketchEditing {
            target,
            input,
            _events: events,
        });
        cx.notify();
    }

    fn edit_value(&self, target: &EditTarget) -> Option<String> {
        match target {
            EditTarget::Title(name) => Some(self.sketch(name)?.title.clone()),
            EditTarget::Shape(shape) => {
                let sketch = self.sketch(&shape.sketch)?;
                match shape.kind {
                    ShapeKind::Box => sketch.find_box(&shape.id).map(|b| b.label.clone()),
                    ShapeKind::Note => sketch
                        .notes
                        .iter()
                        .find(|n| n.id == shape.id)
                        .map(|n| n.text.clone()),
                    ShapeKind::Frame => sketch
                        .frames
                        .iter()
                        .find(|f| f.id == shape.id)
                        .map(|f| f.label.clone()),
                    ShapeKind::Arrow => sketch
                        .arrows
                        .get(shape.id.parse::<usize>().ok()?)
                        .map(|a| a.label.clone()),
                }
            }
        }
    }

    /// The edit in place done: kept and saved, or given up. A note left
    /// empty goes.
    pub fn finish_sketch_edit(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(editing) = self.sketches.editing.take() else {
            return;
        };
        let text = editing.input.read(cx).value().trim().to_string();
        if !keep {
            cx.notify();
            return;
        }
        let mut renamed: Option<String> = None;
        match editing.target.clone() {
            EditTarget::Title(name) => self.retitle(&name, &text, cx),
            EditTarget::Shape(shape) => self.edit_sketches(
                |sketches| {
                    let Some(sketch) = sketches.iter_mut().find(|s| s.name == shape.sketch) else {
                        return;
                    };
                    match shape.kind {
                        ShapeKind::Box if !text.is_empty() => {
                            renamed = sketch.relabel_box(&shape.id, &text);
                        }
                        ShapeKind::Box => {}
                        ShapeKind::Note if text.is_empty() => {
                            sketch.notes.retain(|n| n.id != shape.id)
                        }
                        ShapeKind::Note => {
                            if let Some(n) = sketch.notes.iter_mut().find(|n| n.id == shape.id) {
                                n.text = text;
                            }
                        }
                        ShapeKind::Frame => {
                            if let Some(f) = sketch.frames.iter_mut().find(|f| f.id == shape.id) {
                                f.label = text;
                            }
                        }
                        ShapeKind::Arrow => {
                            if let Some(a) = shape
                                .id
                                .parse::<usize>()
                                .ok()
                                .and_then(|ix| sketch.arrows.get_mut(ix))
                            {
                                a.label = text;
                            }
                        }
                    }
                },
                cx,
            ),
        }
        // A box's id follows its label: kept selected, and adopted.
        if let (EditTarget::Shape(shape), Some(id)) = (&editing.target, renamed) {
            let new = ShapeRef {
                id: id.clone(),
                ..shape.clone()
            };
            for key in &mut self.selected {
                if *key == shape.key() {
                    *key = new.key();
                }
            }
            for at in self.sketches.adopted.values_mut() {
                if at.0 == shape.sketch && at.1 == shape.id {
                    at.1 = id.clone();
                }
            }
        }
        cx.notify();
    }

    /// Sketch `name` titled `title`: its file renamed after it, when that
    /// name's free.
    fn retitle(&mut self, name: &str, title: &str, cx: &mut Context<Self>) {
        if title.is_empty() {
            return;
        }
        let taken: Vec<String> = self
            .sketches
            .sketches
            .iter()
            .filter(|s| s.name != name)
            .map(|s| s.name.clone())
            .collect();
        let renamed = sketch::unique(&sketch::slug(title, "sketch"), &taken);
        let name = name.to_string();
        let title = title.to_string();
        self.edit_sketches(
            |sketches| {
                if let Some(sketch) = sketches.iter_mut().find(|s| s.name == name) {
                    sketch.title = title;
                    sketch.name = renamed.clone();
                }
            },
            cx,
        );
        if self.sketches.active.as_deref() == Some(name.as_str()) {
            self.sketches.active = Some(renamed.clone());
        }
        let old = format!("{}{name}/", sketch::PREFIX);
        let new = format!("{}{renamed}/", sketch::PREFIX);
        for key in &mut self.selected {
            if let Some(rest) = key.strip_prefix(&old) {
                *key = format!("{new}{rest}");
            }
        }
        self.rematch();
    }

    /// Whether a sketch's field has the keyboard.
    pub fn editing_sketch(&self, window: &Window, cx: &App) -> bool {
        self.sketches
            .editing
            .as_ref()
            .is_some_and(|editing| editing.input.focus_handle(cx).is_focused(window))
    }

    // Moving between levels.

    /// Show sketch `name`: at its level, the camera on it.
    pub fn go_to_sketch(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(sketch) = self.sketch(name).cloned() else {
            return;
        };
        let Some(level) = self.level_named(&sketch.level) else {
            return;
        };
        if level != self.level {
            self.show(level, cx);
        }
        self.sketches.active = Some(sketch.name.clone());
        self.sketches.list_open = false;
        let Some(bounds) = self.lay_out(&sketch).bounds() else {
            return;
        };
        let size = self.canvas_size();
        let pad = 120.;
        let screen = Rect {
            x: pad,
            y: pad,
            width: (size.0 - 2. * pad).max(100.),
            height: (size.1 - 2. * pad).max(100.),
        };
        let mut to = Camera::showing(bounds, screen, size);
        to.zoom = to.zoom.min(self.zoom.max(1.));
        let (x, y) = bounds.centre();
        to.x = x;
        to.y = y;
        self.transition = Some(Transition::pan(self.camera(), to));
        self.set_camera(to);
        self.after_moving(cx);
    }

    /// The sketches to mark at the level showing: drawn elsewhere, on the
    /// boxes here that carry them. Each mark: the box, and the sketches.
    pub(super) fn sketch_marks(&self) -> Vec<(super::lens::Landing, Vec<String>)> {
        let here = self.view_key();
        let mut marks: Vec<(super::lens::Landing, Vec<String>)> = Vec::new();
        for sketch in &self.sketches.sketches {
            if sketch.archived || sketch.level == here {
                continue;
            }
            let mut places: Vec<&str> = sketch.places();
            if let Some(focus) = sketch.level.strip_prefix("plan:") {
                places.push(focus);
            }
            let mut seen = HashSet::new();
            for place in places {
                let Some(landing) = self.landing(place) else {
                    continue;
                };
                if !seen.insert(landing.key.clone()) {
                    continue;
                }
                match marks.iter_mut().find(|(at, _)| at.key == landing.key) {
                    Some((_, names)) if !names.contains(&sketch.name) => {
                        names.push(sketch.name.clone())
                    }
                    Some(_) => {}
                    None => marks.push((landing, vec![sketch.name.clone()])),
                }
            }
        }
        marks
    }

    // Asking, adopting, tasks.

    /// The sketches with a shape selected, for the agent: each in words,
    /// with what it's attached to.
    pub(super) fn sketch_lines(&self) -> Vec<String> {
        let selected: Vec<ShapeRef> = self
            .selected
            .iter()
            .filter_map(|key| ShapeRef::of_key(key))
            .collect();
        let mut names: Vec<String> = Vec::new();
        for shape in &selected {
            if !names.contains(&shape.sketch) {
                names.push(shape.sketch.clone());
            }
        }
        let mut lines = Vec::new();
        for name in names {
            let Some(sketch) = self.sketch(&name) else {
                continue;
            };
            lines.push(self.describe_sketch(sketch, &selected));
        }
        lines
    }

    /// Sketch `sketch` in words for the agent: its boxes (each named
    /// `sketch:<name>/<box>`, new or real), arrows, notes and frames, the
    /// selected ones marked, then the real places it's attached to.
    fn describe_sketch(&self, sketch: &Sketch, selected: &[ShapeRef]) -> String {
        let level = match &sketch.level {
            level if level == "site" => "Site".to_string(),
            level if level == "domain" => "Domain".to_string(),
            level => format!("the Plan of {}", level.trim_start_matches("plan:")),
        };
        let is_selected = |kind: ShapeKind, id: &str| {
            selected
                .iter()
                .any(|s| s.sketch == sketch.name && s.kind == kind && s.id == id)
        };
        let mark = |on: bool| if on { " [selected]" } else { "" };
        let end = |end: &str| match sketch.find_box(end) {
            Some(b) => format!("{} “{}”", sketch::place_name(&sketch.name, &b.id), b.label),
            None => end.to_string(),
        };
        let mut lines = vec![format!(
            "My sketch “{}”, drawn at {level}. It's mine: don't change it. Answer beside it, with map_note on its boxes (by the names below) or on real places, and map_propose with boxes near its boxes for what you'd add or change:",
            sketch.title
        )];
        for b in &sketch.boxes {
            let state = match self.box_match(&sketch.name, &b.id) {
                Match::Real(place) => format!("real: {place}"),
                Match::Unsure(n) => format!("new, its name matches {n} things"),
                Match::None => "new: no code yet".into(),
            };
            let mut line = format!(
                "- box {} “{}” ({state}), drawn by {}",
                sketch::place_name(&sketch.name, &b.id),
                b.label,
                if b.near.is_empty() {
                    "nothing"
                } else {
                    &b.near
                }
            );
            if !b.purpose.is_empty() {
                line.push_str(&format!(" — {}", b.purpose));
            }
            line.push_str(mark(is_selected(ShapeKind::Box, &b.id)));
            lines.push(line);
        }
        for (ix, arrow) in sketch.arrows.iter().enumerate() {
            let real = if self.arrow_real(sketch, arrow) {
                "the code calls along it"
            } else {
                "no such call yet"
            };
            let label = if arrow.label.is_empty() {
                String::new()
            } else {
                format!(" “{}”", arrow.label)
            };
            lines.push(format!(
                "- arrow from {} to {}{label} ({real}){}",
                end(&arrow.from),
                end(&arrow.to),
                mark(is_selected(ShapeKind::Arrow, &ix.to_string()))
            ));
        }
        for note in &sketch.notes {
            let on = if note.on.is_empty() {
                String::new()
            } else {
                format!(" on {}", end(&note.on))
            };
            lines.push(format!(
                "- note{on}: “{}”{}",
                note.text,
                mark(is_selected(ShapeKind::Note, &note.id))
            ));
        }
        for frame in &sketch.frames {
            lines.push(format!(
                "- frame “{}”{}",
                frame.label,
                mark(is_selected(ShapeKind::Frame, &frame.id))
            ));
        }
        let attached = sketch.attached();
        if !attached.is_empty() {
            lines.push("Attached to:".into());
            for place in attached {
                lines.push(format!("- {}", self.describe_place(place)));
            }
        }
        if let Some(task) = sketch
            .task
            .as_ref()
            .and_then(|id| self.sketches.tasks.get(id).map(|t| (id, t)))
        {
            lines.push(format!(
                "Its task: {} (id {}), {} of {} todos done.",
                task.1.title, task.0, task.1.done, task.1.total
            ));
        }
        lines.join("\n")
    }

    /// A real place in a line: its name, and what Site says of it.
    fn describe_place(&self, name: &str) -> String {
        let site = self.place(name).and_then(|place| {
            let ix = self.site_box_of_place(&place)?;
            Some(&self.site_nodes()[ix])
        });
        match site {
            Some(node) => {
                let mut line = format!("{name}: {}", node.detail);
                if let Some(purpose) = &node.purpose {
                    line.push_str(&format!(" — {purpose}"));
                }
                line
            }
            None => name.to_string(),
        }
    }

    /// A box's label, for the list of drawings.
    pub(super) fn sketch_box_label(&self, sketch: &str, id: &str) -> String {
        self.sketch(sketch)
            .and_then(|s| s.find_box(id))
            .map_or_else(|| id.to_string(), |b| b.label.clone())
    }

    /// Where a box of a sketch lands at the level showing: on it, if its
    /// sketch shows here; else on the real box it's drawn by.
    pub(super) fn sketch_landing(&self, sketch: &str, id: &str) -> Option<super::lens::Landing> {
        let found = self.sketch(sketch)?;
        let b = found.find_box(id)?;
        if !found.archived && found.level == self.view_key() {
            let rect = self.shape_rect(&ShapeRef {
                sketch: sketch.into(),
                kind: ShapeKind::Box,
                id: id.into(),
            })?;
            return Some(super::lens::Landing {
                key: sketch::place_name(sketch, id),
                rect,
            });
        }
        self.landing(&b.near)
    }

    /// Select the whole of sketch `name`, so asking is about all of it.
    pub fn select_sketch(&mut self, name: &str) {
        if let Some(sketch) = self.sketch(name) {
            self.selected = shapes_of(sketch).into_iter().map(|s| s.key()).collect();
        }
    }

    /// Make Task: the agent asked to write a task from sketch `name`, for
    /// the user to agree to before it's saved, linking back to the sketch.
    pub fn make_task(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(sketch) = self.sketch(name).cloned() else {
            return;
        };
        self.select_sketch(name);
        let link = sketch::link(&sketch.name);
        cx.emit(MapEvent::Converse(
            format!("Task from “{}”", sketch.title),
            format!(
                "Turn my sketch “{}” into a task for Jig's task planner. Read the code it touches first. Then write the task in your reply for me to agree to: a short title; a one or two sentence summary, then ### Background, ### Context, ### Purpose and ### Risks, a sentence or two each; and todos, one short line each, in an order that builds the sketch's boxes and arrows, linking the code each touches as [`name`](path#L12) and naming each new box as the sketch labels it. End the description with the line `Sketch: [{}]({link})`. Ask me whether to save it, and only once I agree, save it with create_task. Don't draw on the Map this time.",
                sketch.title, sketch.title
            ),
        ));
    }

    /// Implement: the sketch's task run by the agent, every edit reviewed.
    pub fn implement_sketch(&mut self, name: &str, cx: &mut Context<Self>) {
        let Some(sketch) = self.sketch(name) else {
            return;
        };
        match &sketch.task {
            Some(task) => cx.emit(MapEvent::Implement(task.clone(), sketch.name.clone())),
            None => self.say("Make a task of the sketch first.", cx),
        }
    }

    /// Archive or restore sketch `name`.
    pub fn set_archived(&mut self, name: &str, archived: bool, cx: &mut Context<Self>) {
        let name = name.to_string();
        self.edit_sketches(
            |sketches| {
                if let Some(sketch) = sketches.iter_mut().find(|s| s.name == name) {
                    sketch.archived = archived;
                }
            },
            cx,
        );
        if archived {
            self.selected
                .retain(|key| ShapeRef::of_key(key).is_none_or(|s| s.sketch != name));
        }
    }

    /// Delete sketch `name` (⌘Z brings it back).
    pub fn delete_sketch(&mut self, name: &str, cx: &mut Context<Self>) {
        let name = name.to_string();
        self.edit_sketches(|sketches| sketches.retain(|s| s.name != name), cx);
    }

    /// Adopt: the conversation's proposal `id` (or just its ghost named
    /// `only`) made the user's sketch: each ghost a box where it's drawn,
    /// its arrows between them and real places. Whether all of it is now.
    pub fn adopt(&mut self, id: usize, only: Option<&str>, cx: &mut Context<Self>) -> bool {
        let Some(drawing) = self
            .lens
            .drawings
            .iter()
            .find(|(at, _)| *at == id)
            .map(|(_, d)| d.clone())
        else {
            return false;
        };
        if drawing.kind != LensKind::Propose {
            return false;
        }
        let ghosts = self.ghost_rects(&drawing);
        let preferred = drawing
            .ghosts
            .iter()
            .map(|g| g.near.as_str())
            .chain(
                drawing
                    .links
                    .iter()
                    .flat_map(|l| [l.from.as_str(), l.to.as_str()]),
            )
            .find_map(sketch::parse_place)
            .map(|(sketch, _)| sketch);
        let name = self.sketch_for_new_shape(preferred.as_deref());
        if self
            .sketch(&name)
            .is_some_and(|s| s.is_empty() && s.title == UNTITLED)
            && !drawing.title.trim().is_empty()
            && let Some(sketch) = self.sketches.sketches.iter_mut().find(|s| s.name == name)
        {
            sketch.title = drawing.title.trim().to_string();
        }
        let mut made: Vec<(String, SketchBox)> = Vec::new();
        for (ghost, rect) in drawing.ghosts.iter().zip(&ghosts) {
            if only.is_some_and(|only| only != ghost.name)
                || self
                    .sketches
                    .adopted
                    .contains_key(&(id, ghost.name.clone()))
            {
                continue;
            }
            let Some(rect) = rect else {
                continue;
            };
            let (near, at) = self.anchor(*rect);
            made.push((
                ghost.name.clone(),
                SketchBox {
                    id: String::new(),
                    label: ghost.name.clone(),
                    purpose: ghost.purpose.clone(),
                    near,
                    at,
                    size: [round(rect.width), round(rect.height)],
                },
            ));
        }
        if made.is_empty() {
            return false;
        }
        let adopted_before = self.sketches.adopted.clone();
        let mut ids: HashMap<String, String> = HashMap::new();
        let real = |end: &str| self.place(end).is_some();
        let real_ends: HashSet<String> = drawing
            .links
            .iter()
            .flat_map(|l| [l.from.clone(), l.to.clone()])
            .filter(|end| sketch::parse_place(end).is_none() && real(end))
            .collect();
        self.edit_sketches(
            |sketches| {
                let Some(sketch) = sketches.iter_mut().find(|s| s.name == name) else {
                    return;
                };
                for (ghost, mut b) in made {
                    b.id = sketch.new_id(&b.label, "box");
                    ids.insert(ghost, b.id.clone());
                    sketch.boxes.push(b);
                }
                // An end: a ghost adopted now or before, a box of this
                // sketch, or a real place.
                let end = |end: &str| -> Option<String> {
                    if let Some(id) = ids.get(end) {
                        return Some(id.clone());
                    }
                    if let Some((s, b)) = adopted_before.get(&(id, end.to_string()))
                        && *s == name
                    {
                        return Some(b.clone());
                    }
                    if let Some((s, b)) = sketch::parse_place(end) {
                        return (s == name && sketch.is_box(&b)).then_some(b);
                    }
                    real_ends.contains(end).then(|| end.to_string())
                };
                let arrows: Vec<SketchArrow> = drawing
                    .links
                    .iter()
                    .filter(|link| ids.contains_key(&link.from) || ids.contains_key(&link.to))
                    .filter_map(|link| {
                        Some(SketchArrow {
                            from: end(&link.from)?,
                            to: end(&link.to)?,
                            label: link.label.clone(),
                        })
                    })
                    .collect();
                sketch.arrows.extend(arrows);
            },
            cx,
        );
        for (ghost, box_id) in ids {
            self.sketches
                .adopted
                .insert((id, ghost), (name.clone(), box_id));
        }
        self.sketches.active = Some(name);
        let all = drawing
            .ghosts
            .iter()
            .all(|g| self.sketches.adopted.contains_key(&(id, g.name.clone())));
        if all {
            cx.emit(MapEvent::Proposal(id, super::ProposalChoice::Adopt));
        }
        cx.notify();
        all
    }

    /// Where beside `near` (the box of place `name`) there's room: past the
    /// user's shapes on its row, when it's a box of their sketch, so the
    /// agent's ghosts cover none of it.
    fn clear_right_of(&self, name: &str, near: Rect) -> f32 {
        let Some((sketch, _)) = sketch::parse_place(name) else {
            return near.right();
        };
        let Some(found) = self.sketch(&sketch) else {
            return near.right();
        };
        let laid = self.lay_out(found);
        laid.boxes
            .iter()
            .chain(&laid.notes)
            .map(|(_, rect)| *rect)
            .filter(|rect| rect.x >= near.x && rect.y < near.bottom() && rect.bottom() > near.y)
            .map(|rect| rect.right())
            .fold(near.right(), f32::max)
    }

    /// Where each of a proposal's ghosts is drawn now, in the drawing,
    /// stacked beside the place it goes by as the canvas draws them.
    pub(super) fn ghost_rects(&self, drawing: &Drawing) -> Vec<Option<Rect>> {
        let zoom = self.zoom.max(0.0001);
        let (width, height) = super::lens_draw::GHOST;
        let mut stacked: HashMap<String, usize> = HashMap::new();
        drawing
            .ghosts
            .iter()
            .map(|ghost| {
                let near = self.landing(&ghost.near)?.rect;
                let n = stacked.entry(ghost.near.clone()).or_default();
                let rect = Rect {
                    x: self.clear_right_of(&ghost.near, near) + 36. / zoom,
                    y: near.y + *n as f32 * (height + 12.) / zoom,
                    width: width / zoom,
                    height: height / zoom,
                };
                *n += 1;
                Some(rect)
            })
            .collect()
    }
}

/// Whether a sketch is kept: it holds something, or a task was made of it.
fn worth_keeping(sketch: &Sketch) -> bool {
    !sketch.is_empty() || sketch.task.is_some()
}

/// Every shape of `sketch`, as selections name them.
pub(super) fn shapes_of(sketch: &Sketch) -> Vec<ShapeRef> {
    let shape = |kind: ShapeKind, id: String| ShapeRef {
        sketch: sketch.name.clone(),
        kind,
        id,
    };
    sketch
        .boxes
        .iter()
        .map(|b| shape(ShapeKind::Box, b.id.clone()))
        .chain((0..sketch.arrows.len()).map(|ix| shape(ShapeKind::Arrow, ix.to_string())))
        .chain(
            sketch
                .notes
                .iter()
                .map(|n| shape(ShapeKind::Note, n.id.clone())),
        )
        .chain(
            sketch
                .frames
                .iter()
                .map(|f| shape(ShapeKind::Frame, f.id.clone())),
        )
        .collect()
}

/// How far drawing point `at` is from `rect` (0 inside it).
fn distance(rect: Rect, (x, y): (f32, f32)) -> f32 {
    let dx = (rect.x - x).max(0.).max(x - rect.right());
    let dy = (rect.y - y).max(0.).max(y - rect.bottom());
    dx.hypot(dy)
}

/// `rect` grown by `by` on every side (shrunk, for less than 0).
fn grow(rect: Rect, by: f32) -> Rect {
    Rect {
        x: rect.x - by,
        y: rect.y - by,
        width: (rect.width + 2. * by).max(0.),
        height: (rect.height + 2. * by).max(0.),
    }
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
