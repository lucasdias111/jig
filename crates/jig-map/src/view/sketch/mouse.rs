//! The mouse on a sketch: pressing, dragging and letting go.

use super::*;

impl MapView {
    /// A press on the canvas with a tool, or on a sketch's shape: taken
    /// here, or left to the Map.
    pub(in crate::view) fn sketch_mouse_down(
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
            self.drag = Some(super::super::input::Drag::Sketch(SketchDrag::Draw {
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
        self.drag = Some(super::super::input::Drag::Sketch(SketchDrag::Press {
            shape,
            start: position,
            shift: event.modifiers.shift,
        }));
        true
    }

    /// What an arrow would start or end on at `position`: a sketch's box,
    /// else a real box.
    pub(super) fn end_at(&self, position: Point<Pixels>) -> Option<End> {
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
    pub(in crate::view) fn sketch_mouse_move(
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
                if super::super::input::far(position, start) && shape.kind != ShapeKind::Arrow =>
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
    pub(in crate::view) fn sketch_mouse_up(
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
}
