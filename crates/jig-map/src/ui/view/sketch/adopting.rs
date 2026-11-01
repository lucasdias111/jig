//! What's done with a sketch: a task made from it, Implement, Archive, and adopting the agent's proposals into it.

use super::*;

impl MapView {
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
            cx.emit(MapEvent::Proposal(id, super::super::ProposalChoice::Adopt));
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
    pub(in crate::ui::view) fn ghost_rects(&self, drawing: &Drawing) -> Vec<Option<Rect>> {
        let zoom = self.zoom.max(0.0001);
        let (width, height) = super::super::lens_draw::GHOST;
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
