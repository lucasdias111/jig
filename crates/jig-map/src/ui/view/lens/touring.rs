//! Touring a trace, the camera following across levels.

use super::*;

impl MapView {
    /// Start touring the drawing `source` from its first step.
    pub fn start_tour(&mut self, source: Source, cx: &mut Context<Self>) {
        if self.drawing(source).is_none_or(|d| d.steps.is_empty()) {
            return;
        }
        self.lens.tour = Some(Tour { source, step: 0 });
        self.fly_to_step(cx);
    }

    /// ←/→: the tour's step before or after, the camera following. Without
    /// a tour, → starts one on the only trace showing. Whether it moved.
    pub fn tour_by(&mut self, delta: i32, cx: &mut Context<Self>) -> bool {
        let Some(tour) = self.lens.tour else {
            let traces: Vec<Source> = self
                .drawings_showing()
                .into_iter()
                .filter(|(_, d)| d.kind.stepped())
                .map(|(source, _)| source)
                .collect();
            return match traces.as_slice() {
                [only] if delta > 0 => {
                    self.start_tour(*only, cx);
                    true
                }
                _ => false,
            };
        };
        let Some(steps) = self.drawing(tour.source).map(|d| d.steps.len()) else {
            return false;
        };
        let step = tour.step as i32 + delta;
        if step < 0 || step >= steps as i32 {
            return false;
        }
        self.lens.tour = Some(Tour {
            step: step as usize,
            ..tour
        });
        self.fly_to_step(cx);
        true
    }

    /// Esc on a tour: it ends, its drawing staying.
    pub fn end_tour(&mut self, cx: &mut Context<Self>) -> bool {
        let ended = self.lens.tour.take().is_some();
        if ended {
            cx.notify();
        }
        ended
    }

    pub(super) fn fly_to_step(&mut self, cx: &mut Context<Self>) {
        let Some(tour) = self.lens.tour else {
            return;
        };
        let Some(place) = self
            .drawing(tour.source)
            .and_then(|d| d.steps.get(tour.step))
            .map(|step| step.place.clone())
        else {
            return;
        };
        self.fly_to(&place, cx);
    }

    /// Fly the Map to the place named `name`, crossing levels where the
    /// one showing doesn't hold it closely: a function to its box on the
    /// Plan, else its Plan; anything else to its box at Site, open. False
    /// when it names no place.
    pub fn fly_to(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let Some(place) = self.place(name) else {
            return false;
        };
        let Some(project) = self.project.clone() else {
            return false;
        };
        if let Place::Sketch { sketch, .. } = &place {
            self.go_to_sketch(sketch, cx);
            return true;
        }
        if let Level::Plan(_) = self.level {
            if let Some(landing) = self.plan_landing(&place)
                && (place.function_in(&project).is_none() || landing.key.starts_with("fn:"))
            {
                self.selected = vec![landing.key.clone()];
                self.pan_to(landing.rect, cx);
                return true;
            }
            if let Some(id) = place.function_in(&project) {
                self.show(Level::Plan(Focus::Function(project.symbol(id))), cx);
                return true;
            }
        }
        // A function from Site or Domain: its Plan only when the tour or
        // link is about it; Site shows the box holding it.
        let Some(ix) = self.site_box_of_place(&place) else {
            return false;
        };
        match self.level {
            Level::Site => {
                let to = self.camera_on_site_box(ix);
                self.selected = vec![site_key(&self.site_nodes()[ix].key)];
                self.transition = Some(Transition::pan(self.camera(), to));
                self.set_camera(to);
                self.after_moving(cx);
            }
            Level::Domain | Level::Plan(_) => {
                let nodes = self.site_nodes();
                let region = nodes[ix].region;
                // A region itself stays at Domain, selected.
                if ix == region && self.level == Level::Domain {
                    self.selected = vec![nodes[ix].key.clone()];
                    cx.notify();
                    return true;
                }
                let from = nodes[region].rect;
                self.arriving_at = Some((from, vec![ix]));
                self.show(Level::Site, cx);
            }
        }
        true
    }

    /// Glide the camera so drawing rect `rect` is in the middle.
    pub(in crate::ui::view) fn pan_to(&mut self, rect: Rect, cx: &mut Context<Self>) {
        let from = self.camera();
        let (x, y) = rect.centre();
        let to = super::super::zoom::Camera { x, y, ..from };
        self.transition = Some(Transition::pan(from, to));
        self.set_camera(to);
        self.after_moving(cx);
    }
}
