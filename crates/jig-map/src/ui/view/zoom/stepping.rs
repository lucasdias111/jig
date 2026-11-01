//! Zooming within a level and stepping between levels.

use super::*;

impl MapView {
    /// Zooming stays put while a gesture that changed level goes on.
    pub(super) fn locked(&mut self, now: Instant) -> bool {
        let fading = self
            .transition
            .as_ref()
            .is_some_and(|transition| transition.fades() && transition.locks);
        let locked = fading
            || self
                .zoom_lock
                .is_some_and(|at| now.duration_since(at) < GESTURE_GAP);
        if locked {
            self.zoom_lock = Some(now);
        }
        locked
    }

    /// A pinch or ⌘-scroll by `factor` around `around`: zoomed within the
    /// level, or past its ends to the next.
    pub(in crate::ui::view) fn zoom_at(
        &mut self,
        factor: f32,
        around: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let now = Instant::now();
        if self.locked(now) {
            return;
        }
        if self.transition.is_some() {
            self.transition = None;
        }
        let wanted = self.zoom * factor;
        match self.level.clone() {
            Level::Plan(_) => {
                if factor < 1. && wanted < self.plan_out() {
                    return self.step_out(cx);
                }
                if factor > 1.
                    && wanted > PLAN_IN
                    && let Some(ix) = self.plan_box_at(around)
                    && (self.plan.nodes[ix].function().is_some()
                        || matches!(self.plan.nodes[ix].item, Item::File { .. }))
                {
                    return self.enter_plan_box(ix, cx);
                }
                self.zoom_by(factor, around, PLAN_MIN, PLAN_MAX);
            }
            Level::Domain => {
                if factor > 1. && self.zoom_into_domain(wanted, around, cx) {
                    return;
                }
                let min = self.domain_min();
                self.zoom_by(factor, around, min, super::super::domain::DOMAIN_MAX);
            }
            Level::Site => {
                if factor < 1. && self.leaves_site(wanted, around) {
                    self.zoom_by(factor, around, 0., SITE_MAX);
                    self.continuous = true;
                    return self.show(Level::Domain, cx);
                }
                if factor > 1.
                    && let Some(ix) = self.site_box_at(around)
                {
                    let node = &self.site_nodes()[ix];
                    let (width, _) = self.canvas_size();
                    if has_plan(node.kind, node.functions)
                        && node.rect.width * wanted > SITE_IN * width
                    {
                        let place = node.place.clone();
                        return self.show(Level::Plan(Focus::Module(place)), cx);
                    }
                }
                self.zoom_by(factor, around, 0., SITE_MAX);
            }
        }
        self.moved(cx);
        cx.notify();
    }

    /// Zoomed out at Site to `zoom` around `around`: Domain once the region
    /// there is closing, or, between regions, once none in view is open.
    pub(super) fn leaves_site(&self, zoom: f32, around: Point<Pixels>) -> bool {
        let at = self.to_drawing(around);
        let region = self.site.as_ref().and_then(|site| {
            site.regions()
                .map(|ix| &site.nodes[ix])
                .find(|node| node.opens() && node.rect.contains(at))
        });
        match region {
            Some(node) => zoom < node.open_at * SITE_TO_DOMAIN,
            None => !self.any_region_open(zoom),
        }
    }

    /// ⌘+ and ⌘-: a step in or out around the middle, gliding there, and
    /// on past a level's end as a pinch would go.
    pub fn zoom_step(&mut self, closer: bool, cx: &mut Context<Self>) {
        let from = self.camera();
        let level = self.level.clone();
        let factor = if closer { KEY_STEP } else { 1. / KEY_STEP };
        self.zoom_at(factor, self.middle(), cx);
        if self.level == level && self.transition.is_none() {
            let to = self.camera();
            if to != from {
                self.transition = Some(Transition::pan(from, to));
                cx.notify();
            }
        }
    }

    /// ⌘0: the level as it first shows.
    pub fn zoom_reset(&mut self, cx: &mut Context<Self>) {
        let to = self.default_camera();
        self.transition = Some(Transition::pan(self.camera(), to));
        self.set_camera(to);
        self.save_view();
        cx.notify();
    }

    /// ⌘↑: Plan out to Site, Site out to Domain; Domain out to all of it.
    pub fn step_out(&mut self, cx: &mut Context<Self>) {
        match self.level {
            Level::Plan(_) => self.show(Level::Site, cx),
            Level::Site => self.show(Level::Domain, cx),
            Level::Domain => {
                let to = self.domain_fit();
                self.transition = Some(Transition::pan(self.camera(), to));
                self.set_camera(to);
                self.save_view();
                cx.notify();
            }
        }
    }

    /// ⌘↓ or Enter: into the box selected, else the one in the middle.
    pub fn step_in(&mut self, cx: &mut Context<Self>) {
        match self.level {
            Level::Plan(_) => {
                let picked = match self.selected_plan().as_slice() {
                    [ix] => Some(*ix),
                    _ => self.plan.focus,
                };
                if let Some(ix) = picked {
                    self.enter_plan_box(ix, cx);
                }
            }
            Level::Domain => self.step_into_domain(cx),
            Level::Site => {
                let picked = self
                    .selected
                    .iter()
                    .find_map(|key| self.site.as_ref()?.index_of(key.strip_prefix("site:")?))
                    .or_else(|| {
                        let (width, height) = self.canvas_size();
                        let middle =
                            self.bounds.get().origin + point(px(width / 2.), px(height / 2.));
                        self.site_box_at(middle)
                    });
                if let Some(ix) = picked {
                    self.enter_site_box(ix, cx);
                }
            }
        }
    }

    /// Into Site box `ix`: a folder flown into until it's open and fills
    /// the view; a file or a type to the Plan of its functions; a file
    /// the index doesn't read opened as it is.
    pub fn enter_site_box(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(node) = self.site_nodes().get(ix).cloned() else {
            return;
        };
        match node.kind {
            _ if has_plan(node.kind, node.functions) => {
                self.show(Level::Plan(Focus::Module(node.place)), cx)
            }
            Kind::File { .. } | Kind::Type => {
                let path = node.place.split('#').next().unwrap_or(&node.place);
                cx.emit(MapEvent::OpenFile(self.root.join(path)));
            }
            _ => {
                let to = self.camera_on_site_box(ix);
                self.transition = Some(Transition::pan(self.camera(), to));
                self.set_camera(to);
                self.selected = vec![super::super::site_key(&node.key)];
                self.save_view();
                cx.notify();
            }
        }
    }

    /// Scrolled or zoomed: kept once it settles.
    pub(in crate::ui::view) fn moved(&mut self, cx: &mut Context<Self>) {
        let at = Instant::now();
        self.scrolled = Some(at);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(600))
                .await;
            this.update(cx, |this, _| {
                if this.scrolled == Some(at) {
                    this.scrolled = None;
                    this.save_view();
                }
            })
            .ok();
        })
        .detach();
    }
}
