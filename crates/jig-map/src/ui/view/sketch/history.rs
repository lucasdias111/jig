//! Editing sketches: every change saved at once, and undone with ⌘Z.

use super::*;

impl MapView {
    /// Change the sketches: remembered for ⌘Z, saved, matched again.
    pub(in crate::ui::view) fn edit_sketches(
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
    pub(super) fn persist(&mut self, before: &[Sketch]) {
        for sketch in &self.sketches.sketches {
            if !worth_keeping(sketch)
                || before
                    .iter()
                    .any(|was| was == sketch && was.name == sketch.name)
            {
                continue;
            }
            if let Err(error) = crate::store::persist::sketches::save(&self.root, sketch) {
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
                let _ = crate::store::persist::sketches::remove(&self.root, &was.name);
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
    pub(super) fn sketch_for_new_shape(&mut self, preferred: Option<&str>) -> String {
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
}
