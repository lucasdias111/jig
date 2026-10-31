//! Laying sketches out beside the real boxes they're placed by, and matching their boxes and arrows to code.

use super::*;

impl MapView {
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
    pub(super) fn placed(&self, near: &str, at: [f32; 2], size: [f32; 2]) -> Rect {
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
    pub(in crate::view) fn lay_out(&self, sketch: &Sketch) -> Laid {
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
    pub(in crate::view) fn sketches_bounds(&self) -> Option<Rect> {
        Rect::around(self.laid_sketches().iter().filter_map(Laid::bounds))
    }

    /// Every sketch at the level showing, laid out.
    pub(in crate::view) fn laid_sketches(&self) -> Vec<Laid> {
        self.sketches_here().map(|s| self.lay_out(s)).collect()
    }

    /// What box `id` of sketch `name` matches in the code.
    pub(in crate::view) fn box_match(&self, name: &str, id: &str) -> Match {
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
    pub(in crate::view) fn rematch(&mut self) {
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
    pub(in crate::view) fn arrow_real(&self, sketch: &Sketch, arrow: &SketchArrow) -> bool {
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
}
