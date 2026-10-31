//! Editing a shape's label, or a sketch's title, in place.

use super::*;

impl MapView {
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
}
