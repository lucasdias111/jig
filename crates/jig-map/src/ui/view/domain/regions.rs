//! Placing Domain's regions, zooming into one, and which one is selected.

use super::*;

impl MapView {
    /// The whole Domain level in view.
    pub(in crate::ui::view) fn domain_fit(&self) -> Camera {
        let (width, height) = self.canvas_size();
        let Some(bounds) = self.domain_level().and_then(|level| level.bounds()) else {
            return Camera {
                x: 0.,
                y: 0.,
                zoom: 1.,
            };
        };
        // Room above for the banner, and the top row's names.
        let top = 28.
            + if self.domain_banner().is_some() {
                56. + super::super::ruler::RULER_ROOM - 12.
            } else {
                24.
            };
        let zoom = ((width - 60.) / bounds.width.max(1.))
            .min((height - 40. - top) / bounds.height.max(1.))
            .min(1.);
        let (x, y) = bounds.centre();
        Camera {
            x,
            y: y - (top - 24.) / 2. / zoom,
            zoom,
        }
    }

    /// Domain as `camera` sees it: each region's name, purpose and offers,
    /// and one link between each pair that has flows.
    pub(in crate::ui::view) fn domain_scene(&self, _camera: Camera) -> Scene {
        let mut scene = Scene::default();
        let Some(level) = self.domain_level() else {
            return scene;
        };
        for (ix, _) in level.showing() {
            scene.regions.push(self.region_box(&level, ix, 0.));
        }
        // What flows along each link: in full on the selected domain's,
        // faintly on the rest. The full ones are placed first, so the faint
        // ones keep out of their way.
        let picked = self.selected_region();
        let mut links: Vec<Arrow> = level
            .links
            .iter()
            .map(|link| picked_link(&level, link, picked.as_deref()))
            .collect();
        links.sort_by_key(|arrow| arrow.faint);
        scene.arrows.extend(links);
        scene
    }

    /// Region `ix` of the level as drawn, opened `open` of the way.
    pub(in crate::ui::view) fn region_box(
        &self,
        level: &DomainLevel,
        ix: usize,
        open: f32,
    ) -> RegionBox {
        let region = &level.regions[ix];
        RegionBox {
            key: region.key.clone(),
            rect: region.rect,
            header: region.header,
            name: region.name.clone(),
            purpose: region.purpose.clone(),
            offers: region.offers.clone(),
            why: region.why.clone(),
            note: region.detail(),
            kind: region.kind,
            draft: level.draft,
            proposed: self.domains.proposal.is_some()
                && matches!(region.kind, RegionKind::Domain(_)),
            open,
            lit: self.selected_region().as_deref() == Some(region.key.as_str()),
            alpha: 1.,
            ix,
        }
    }

    /// The region under `position`, or whose name is, by its place in the
    /// level.
    pub(in crate::ui::view) fn region_at(&self, position: Point<Pixels>) -> Option<usize> {
        let at = self.to_drawing(position);
        let level = self.domain_level()?;
        level
            .showing()
            .find(|(_, region)| region.rect.contains(at) || self.label_rect(region).contains(at))
            .map(|(ix, _)| ix)
    }

    /// Where region `region`'s name is drawn, above it, in the drawing.
    pub(in crate::ui::view) fn label_rect(
        &self,
        region: &crate::domain::domains::level::Region,
    ) -> Rect {
        let quiet = !matches!(region.kind, RegionKind::Domain(_));
        let zoom = self.zoom.max(0.0001);
        let room = super::super::domain_draw::name_room(region.rect.width * zoom, zoom);
        let name = super::super::domain_draw::name_label(&region.name, quiet, room);
        Rect {
            x: region.rect.x,
            y: region.rect.y - name.height / zoom,
            width: name.width / zoom,
            height: name.height / zoom,
        }
    }

    /// Zoomed in at Domain far enough over a region that it opens: into
    /// Site, carrying on where it is. Whether it went.
    pub(in crate::ui::view) fn zoom_into_domain(
        &mut self,
        wanted: f32,
        around: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(ix) = self.region_at(around) else {
            return false;
        };
        let level = self.domain_level().expect("a region");
        let Some(site) = self.site.clone() else {
            return false;
        };
        let Some(node) = site.index_of(&level.regions[ix].key) else {
            return false;
        };
        if site.nodes[node].opens()
            && wanted >= site.nodes[node].open_at * super::super::zoom::DOMAIN_TO_SITE
        {
            self.zoom_by(wanted / self.zoom, around, 0., f32::MAX);
            self.continuous = true;
            self.show(Level::Site, cx);
            return true;
        }
        false
    }

    /// ⌘↓ or Enter at Domain: into the region selected, else the one in
    /// the middle.
    pub(in crate::ui::view) fn step_into_domain(&mut self, cx: &mut Context<Self>) {
        let Some(level) = self.domain_level() else {
            return;
        };
        let picked = self
            .selected
            .iter()
            .find_map(|key| level.regions.iter().position(|region| &region.key == key))
            .or_else(|| self.region_at(self.middle()));
        if let Some(ix) = picked {
            self.enter_region(ix, cx);
        }
    }

    /// Into Site from region `ix`: it fills the view, open.
    pub(in crate::ui::view) fn enter_region(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(level) = self.domain_level() else {
            return;
        };
        let region = &level.regions[ix];
        let boxes: Vec<usize> = self
            .site
            .as_ref()
            .and_then(|site| site.index_of(&region.key))
            .into_iter()
            .collect();
        self.arriving_at = Some((region.rect, boxes));
        self.show(Level::Site, cx);
    }

    /// Site showing all of `boxes`: one box as entering it does, several
    /// fitting the view.
    pub(in crate::ui::view) fn camera_on_site_boxes(&self, boxes: &[usize]) -> Camera {
        match boxes {
            [] => self.default_camera(),
            [one] => self.camera_on_site_box(*one),
            _ => {
                let (width, height) = self.canvas_size();
                let rect = self.site_boxes_rect(boxes).expect("boxes");
                let screen = Rect {
                    x: 40.,
                    y: 40.,
                    width: width - 80.,
                    height: height - 80.,
                };
                let to = Camera::showing(rect, screen, (width, height));
                Camera {
                    zoom: to.zoom.clamp(self.site_min(), 3.),
                    ..to
                }
            }
        }
    }

    /// Coming out to Domain from `left`, seen by `camera`: the region
    /// holding what was selected or focused there, selected (you are
    /// here); from Site with nothing selected, the region in the middle.
    pub(in crate::ui::view) fn domain_of_selection_at(
        &self,
        left: &Level,
        camera: Camera,
    ) -> Vec<String> {
        let Some(level) = self.domain_level() else {
            return Vec::new();
        };
        let place = match left {
            Level::Plan(Focus::Function(symbol)) => Some(symbol.file.clone()),
            Level::Plan(Focus::Module(place)) => Some(place.clone()),
            Level::Site => {
                let site = self.site.as_ref();
                let picked = self
                    .selected
                    .iter()
                    .find_map(|key| site?.index_of(key.strip_prefix("site:")?))
                    .or_else(|| {
                        let site = site?;
                        site.regions()
                            .find(|&ix| site.nodes[ix].rect.contains((camera.x, camera.y)))
                    });
                if let Some(ix) = picked {
                    let region = self.site_nodes()[ix].region;
                    let key = &self.site_nodes()[region].key;
                    if level.regions.iter().any(|r| &r.key == key) {
                        return vec![key.clone()];
                    }
                }
                None
            }
            Level::Domain => None,
        };
        let Some(place) = place else {
            return Vec::new();
        };
        let region = level.region_of(&place).or_else(|| {
            // A folder: the region holding the most of it.
            let mut counts: HashMap<usize, usize> = HashMap::new();
            for (ix, region) in level.regions.iter().enumerate() {
                for member in &region.members {
                    if member.place.starts_with(&place) {
                        *counts.entry(ix).or_default() += 1;
                    }
                }
            }
            counts
                .into_iter()
                .max_by_key(|&(ix, n)| (n, std::cmp::Reverse(ix)))
                .map(|(ix, _)| ix)
        });
        region
            .map(|ix| vec![level.regions[ix].key.clone()])
            .unwrap_or_default()
    }

    /// The domain picked at Site, by its region's key: the one selected
    /// (there, or at Domain before zooming in), or the one holding the box
    /// selected.
    pub(in crate::ui::view) fn selected_region(&self) -> Option<String> {
        let site = self.site.as_ref()?;
        self.selected.iter().find_map(|key| {
            let ix = site.index_of(key.strip_prefix("site:").unwrap_or(key))?;
            Some(site.nodes[site.nodes[ix].region].key.clone())
        })
    }
}
