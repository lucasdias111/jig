//! Drawing shapes with the tools, and deleting them.

use super::*;

impl MapView {
    /// A tool's drag let go: its shape made, and its label edited.
    pub(super) fn draw_shape(
        &mut self,
        tool: Tool,
        from: Point<Pixels>,
        to: Point<Pixels>,
        start: Option<End>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sketches.tool = Tool::Pointer;
        let click = !super::super::input::far_by(from, to, CLICK);
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
    pub(in crate::ui::view) fn delete_selected_shapes(&mut self, cx: &mut Context<Self>) -> bool {
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
}
