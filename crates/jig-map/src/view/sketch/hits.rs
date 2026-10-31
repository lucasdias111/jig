//! Which shape, or which real box, is under the pointer.

use super::*;

impl MapView {
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
    pub(super) fn real_place_at(&self, position: Point<Pixels>) -> Option<String> {
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
    pub(super) fn anchor(&self, rect: Rect) -> (String, [f32; 2]) {
        match self.nearest_real(rect.centre()) {
            Some((name, anchor)) => (name, [round(rect.x - anchor.x), round(rect.y - anchor.y)]),
            None => (String::new(), [round(rect.x), round(rect.y)]),
        }
    }

    /// A drawing size `screen` points big on screen now.
    pub(super) fn drawing_size(&self, (width, height): (f32, f32)) -> [f32; 2] {
        let zoom = self.zoom.max(0.0001);
        [round(width / zoom), round(height / zoom)]
    }

    /// The sketch shape under `position`: a note, a box, an arrow, then a
    /// frame's label or edge.
    pub(in crate::view) fn sketch_at(&self, position: Point<Pixels>) -> Option<ShapeRef> {
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
                let (a, b) = super::super::scene::edge_points(*from, *to);
                super::super::input::distance_to_bow(a, b, at) < reach
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
    pub(in crate::view) fn shape_rect(&self, shape: &ShapeRef) -> Option<Rect> {
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
}
