//! Where a place lands at the level showing.

use super::*;

impl MapView {
    /// The place named `name`, as the project has it.
    pub(in crate::ui::view) fn place(&self, name: &str) -> Option<Place> {
        // The user's sketches change as they draw: never kept.
        if let Some((sketch, shape)) = crate::domain::sketch::parse_place(name) {
            let found = self.sketch(&sketch)?;
            let shape = found
                .find_box(&shape)
                .or_else(|| found.boxes.iter().find(|b| b.label == shape))?;
            return Some(Place::Sketch {
                sketch: found.name.clone(),
                shape: shape.id.clone(),
            });
        }
        if let Some(found) = self.lens.resolved.borrow().get(name) {
            return found.clone();
        }
        let project = self.project.as_ref()?;
        let domains = self.shown_domains();
        let found = resolve(project, domains.as_ref(), &Folder(&self.root), name).ok();
        self.lens
            .resolved
            .borrow_mut()
            .insert(name.to_string(), found.clone());
        found
    }

    /// The box place `name` lands on at the level showing, if it has one.
    pub(in crate::ui::view) fn landing(&self, name: &str) -> Option<Landing> {
        let place = self.place(name)?;
        if let Place::Sketch { sketch, shape } = &place {
            return self.sketch_landing(sketch, shape);
        }
        match &self.level {
            Level::Plan(_) => self.plan_landing(&place),
            Level::Site => {
                let site = self.site.as_ref()?;
                let ix = self.site_box_of_place(&place)?;
                let zoom = self.zoom;
                // Outwards in, as far as the boxes it's in are open.
                let ancestry: Vec<usize> = site.ancestry(ix).collect();
                let mut shown = *ancestry.last()?;
                for &inner in ancestry.iter().rev().skip(1) {
                    if site.nodes[shown].opening(zoom) >= 0.5 {
                        shown = inner;
                    } else {
                        break;
                    }
                }
                let node = &site.nodes[shown];
                Some(Landing {
                    key: node.key.clone(),
                    rect: node.rect,
                })
            }
            Level::Domain => {
                let site = self.site.as_ref()?;
                let ix = self.site_box_of_place(&place)?;
                let key = &site.nodes[site.nodes[ix].region].key;
                let level = self.domain_level()?;
                let region = level.regions.iter().find(|r| &r.key == key)?;
                Some(Landing {
                    key: region.key.clone(),
                    rect: region.rect,
                })
            }
        }
    }

    /// The deepest Site box holding `place`.
    pub(in crate::ui::view) fn site_box_of_place(&self, place: &Place) -> Option<usize> {
        let site = self.site.as_ref()?;
        match place {
            Place::Domain(name) => site.index_of(&format!("domain:{name}")),
            place => site.box_of(&place.site_place()?),
        }
    }

    /// At Plan: the function's box, else the frame of the type or file,
    /// else the file's box.
    pub(in crate::ui::view) fn plan_landing(&self, place: &Place) -> Option<Landing> {
        let project = self.project.as_ref()?;
        if let Some(id) = place.function_in(project) {
            let key = function_key(project, id);
            if let Some(ix) = self.plan.index_of(&key) {
                return Some(Landing {
                    key,
                    rect: self.placed.nodes[ix],
                });
            }
        }
        let site = place.site_place()?;
        let file = place.file();
        let frame = self.placed.frames.iter().find(|(frame, _)| {
            let key = &self.plan.frames[*frame].key;
            *key == site
                || file.is_some_and(|file| key == file || key.starts_with(&format!("{file}#")))
        });
        if let Some((frame, rect)) = frame {
            return Some(Landing {
                key: format!("frame:{}", self.plan.frames[*frame].key),
                rect: *rect,
            });
        }
        self.plan
            .nodes
            .iter()
            .enumerate()
            .find_map(|(ix, node)| match &node.item {
                Item::File { path, .. } if Some(path.as_str()) == file => Some(Landing {
                    key: node.key.clone(),
                    rect: self.placed.nodes[ix],
                }),
                _ => None,
            })
    }
}
