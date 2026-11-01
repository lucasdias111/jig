//! Where each level's camera starts, and fitting Site.

use super::*;

impl MapView {
    /// Where the view is first put on a level: Site fits the project; a
    /// function's Plan centres on it; a module's fits what's there.
    pub(in crate::ui::view) fn default_camera(&self) -> Camera {
        let (width, height) = self.canvas_size();
        match &self.level {
            Level::Site => self.site_fit(),
            Level::Domain => self.domain_fit(),
            Level::Plan(Focus::Function(_)) => self.centred_camera(1.),
            Level::Plan(Focus::Module(_)) => match self.placed.bounds() {
                Some(bounds) => {
                    let zoom = ((width - 80.) / bounds.width.max(1.))
                        .min((height - 100.) / bounds.height.max(1.))
                        .clamp(0.4, 1.);
                    let (x, y) = bounds.centre();
                    Camera { x, y, zoom }
                }
                None => Camera {
                    x: 0.,
                    y: 0.,
                    zoom: 1.,
                },
            },
        }
    }

    /// The Plan's function in the middle at `zoom`, else its middle.
    pub(in crate::ui::view) fn centred_camera(&self, zoom: f32) -> Camera {
        let target = self
            .plan
            .focus
            .map(|focus| self.placed.nodes[focus])
            .or_else(|| self.placed.bounds());
        let (x, y) = target.map_or((0., 0.), |rect| rect.centre());
        Camera { x, y, zoom }
    }

    /// Site at its broadest: the region holding what's selected, else the
    /// biggest, filling the view and open.
    pub(super) fn site_fit(&self) -> Camera {
        let Some(site) = self.site.as_ref() else {
            return Camera {
                x: 0.,
                y: 0.,
                zoom: 1.,
            };
        };
        let selected = self
            .selected
            .iter()
            .find_map(|key| site.index_of(key.strip_prefix("site:")?))
            .map(|ix| site.nodes[ix].region)
            .filter(|&ix| ix != 0);
        let biggest = site.regions().max_by(|&a, &b| {
            let area = |ix: usize| site.nodes[ix].rect.width * site.nodes[ix].rect.height;
            area(a).total_cmp(&area(b))
        });
        match selected.or(biggest) {
            Some(region) => self.camera_on_site_box(region),
            None => Camera {
                x: 0.,
                y: 0.,
                zoom: 1.,
            },
        }
    }

    /// How far out Site goes: past where every region in view has closed
    /// it's Domain. All of Site's regions, without a camera.
    pub(in crate::ui::view) fn site_min(&self) -> f32 {
        let Some(site) = self.site.as_ref() else {
            return 0.01;
        };
        site.regions()
            .filter(|&ix| site.nodes[ix].opens())
            .map(|ix| site.nodes[ix].open_at)
            .fold(f32::INFINITY, f32::min)
            .min(SITE_MAX)
    }

    /// Whether any region in view at `zoom` around the camera now has
    /// started opening.
    pub(super) fn any_region_open(&self, zoom: f32) -> bool {
        let Some(site) = self.site.as_ref() else {
            return false;
        };
        let (width, height) = self.canvas_size();
        let camera = self.camera();
        let view = Rect {
            x: camera.x - width / 2. / zoom,
            y: camera.y - height / 2. / zoom,
            width: width / zoom,
            height: height / zoom,
        };
        site.regions().any(|ix| {
            let node = &site.nodes[ix];
            node.rect.overlaps(&view) && node.opening(zoom) > 0.
        })
    }

    /// Site showing box `ix`: a box that opens filling the view, open; a
    /// file or type with the folder it's in, open, so its neighbours are
    /// in view too.
    pub(in crate::ui::view) fn camera_on_site_box(&self, ix: usize) -> Camera {
        let (width, height) = self.canvas_size();
        let nodes = self.site_nodes();
        let node = &nodes[ix];
        let target = if node.opens() {
            ix
        } else {
            node.parent.unwrap_or(0)
        };
        // Open enough for the box and every box it's in.
        let mut need: f32 = 0.;
        let mut at = Some(target);
        while let Some(n) = at {
            if n != 0 {
                need = need.max(nodes[n].open_to() * 1.02);
            }
            at = nodes[n].parent;
        }
        let screen = Rect {
            x: 40.,
            y: 40.,
            width: width - 80.,
            height: height - 80.,
        };
        let fit = Camera::showing(nodes[target].rect, screen, (width, height));
        // No closer than Site's natural size, so what's around shows too.
        let zoom = fit.zoom.min(1.);
        if zoom >= need {
            return Camera { zoom, ..fit };
        }
        // Too big to fit open: centred on the box itself.
        let (x, y) = node.rect.centre();
        Camera {
            x,
            y,
            zoom: need.min(SITE_MAX),
        }
    }
}
