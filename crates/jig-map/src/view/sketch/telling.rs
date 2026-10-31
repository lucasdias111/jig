//! Going to a sketch from other levels, and telling the agent about one.

use super::*;

impl MapView {
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
    pub(in crate::view) fn sketch_marks(&self) -> Vec<(super::super::lens::Landing, Vec<String>)> {
        let here = self.view_key();
        let mut marks: Vec<(super::super::lens::Landing, Vec<String>)> = Vec::new();
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

    /// The sketches with a shape selected, for the agent: each in words,
    /// with what it's attached to.
    pub(in crate::view) fn sketch_lines(&self) -> Vec<String> {
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
    pub(in crate::view) fn sketch_box_label(&self, sketch: &str, id: &str) -> String {
        self.sketch(sketch)
            .and_then(|s| s.find_box(id))
            .map_or_else(|| id.to_string(), |b| b.label.clone())
    }

    /// Where a box of a sketch lands at the level showing: on it, if its
    /// sketch shows here; else on the real box it's drawn by.
    pub(in crate::view) fn sketch_landing(
        &self,
        sketch: &str,
        id: &str,
    ) -> Option<super::super::lens::Landing> {
        let found = self.sketch(sketch)?;
        let b = found.find_box(id)?;
        if !found.archived && found.level == self.view_key() {
            let rect = self.shape_rect(&ShapeRef {
                sketch: sketch.into(),
                kind: ShapeKind::Box,
                id: id.into(),
            })?;
            return Some(super::super::lens::Landing {
                key: sketch::place_name(sketch, id),
                rect,
            });
        }
        self.landing(&b.near)
    }
}
