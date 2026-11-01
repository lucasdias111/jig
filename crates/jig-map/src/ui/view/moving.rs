//! Moving between levels: stepping out and in, entering a box, re-centring a Plan.

use super::*;

/// Whether `kind` is a box that opens into a Plan: of its functions, or
/// of a file the index can't read as a whole.
pub(super) fn has_plan(kind: Kind, functions: usize) -> bool {
    match kind {
        Kind::File { read: false } => true,
        Kind::File { read: true } | Kind::Type => functions > 0,
        _ => false,
    }
}

impl MapView {
    /// Show `level` without remembering where it was: for Go Back and Go
    /// Forward themselves. Another level cross-fades in; another Plan's
    /// boxes glide.
    pub fn go_to(&mut self, level: Level, cx: &mut Context<Self>) {
        if level == self.level {
            cx.notify();
            return;
        }
        let both_plans = matches!((&self.level, &level), (Level::Plan(_), Level::Plan(_)));
        if both_plans {
            self.recentre(level, cx);
        } else {
            self.step_to(level, cx);
        }
    }

    /// Arrived from the code at `offset` in `path`: the Plan of the
    /// function there, flown out of it with it selected (you are here),
    /// else of its file.
    pub fn arrive_from_code(&mut self, path: &Path, offset: usize, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let relative = debug::relative(&self.root, path);
        let level = match project.function_at(&relative, offset) {
            Some(id) => Level::Plan(Focus::Function(project.symbol(id))),
            None => Level::Plan(Focus::Module(slashed(&relative))),
        };
        if level != self.level {
            self.save_view();
            self.level = level;
            self.load_view();
            self.rebuild(false);
        }
        self.selected = self
            .plan
            .focus
            .map(|focus| vec![self.plan.nodes[focus].key.clone()])
            .unwrap_or_default();
        let to = self.default_camera();
        self.set_camera(to);
        self.transition = Some(Transition::from_code(to));
        self.after_moving(cx);
    }

    /// Another Plan: the box re-centred on stays put, the boxes still
    /// there glide to their new places, new ones slide in and those out
    /// of range fade where they were.
    pub(super) fn recentre(&mut self, level: Level, cx: &mut Context<Self>) {
        let leaving = self.scene_now();
        let was = self.positions();
        self.save_view();
        self.level = level;
        let saved = self.load_view();
        self.rebuild(false);
        // Keep the box re-centred on where it was, the rest relative to it.
        if self.places.is_empty()
            && let Some(focus) = self.plan.focus
            && let Some(&(x, y)) = was.get(&self.plan.nodes[focus].key)
        {
            let now = self.placed.nodes[focus];
            self.placed.shift((x - now.x, y - now.y));
        }
        if let Some(focus) = self.plan.focus {
            self.selected = vec![self.plan.nodes[focus].key.clone()];
        }
        let kept: Vec<String> = self.plan.nodes.iter().map(|n| n.key.clone()).collect();
        let mut gone = leaving;
        gone.cards.retain(|card| !kept.contains(&card.key));
        gone.arrows.clear();
        gone.frames.clear();
        self.leaving = Some((gone, Instant::now()));
        self.glide = Some((was, Instant::now()));
        let from = self.camera();
        let to = match saved {
            Some(camera) => camera,
            None => self.centred_camera(from.zoom),
        };
        self.transition = Some(Transition::pan(from, to));
        self.set_camera(to);
        self.after_moving(cx);
    }

    /// To another level, cross-fading: the box entered grows into its
    /// contents, or what's left shrinks into the box holding it.
    pub(super) fn step_to(&mut self, level: Level, cx: &mut Context<Self>) {
        let old_scene = self.scene_now();
        let old_camera = self.camera();
        let left = self.level.clone();
        // What the old level was showing, in its own drawing.
        let old_content = self.content_rect();
        let entered = self.entered_rect(&level);
        let continuous = std::mem::take(&mut self.continuous);
        self.save_view();
        self.level = level;
        let saved = self.load_view();
        self.rebuild(false);
        let same = self.content_rect().map(|rect| (rect, rect));
        let (to, matched) = match (&left, &self.level) {
            // Zoomed out of Site: Domain is the same drawing, the regions
            // closing where they are.
            (Level::Site, Level::Domain) if continuous => {
                self.selected = self.domain_of_selection_at(&left, old_camera);
                let to = Camera {
                    zoom: old_camera.zoom.clamp(self.domain_min(), domain::DOMAIN_MAX),
                    ..old_camera
                };
                (to, same)
            }
            // Out to Domain: the region holding what was selected, picked.
            (_, Level::Domain) => {
                self.selected = self.domain_of_selection_at(&left, old_camera);
                let to = saved.unwrap_or_else(|| self.default_camera());
                (to, None)
            }
            // Zoomed into a region: Site carries on where Domain was.
            (Level::Domain, Level::Site) if continuous => {
                self.arriving_at = None;
                (old_camera, same)
            }
            // Into Site from a domain: what it stands for.
            (Level::Domain, Level::Site) => match self.arriving_at.take() {
                Some((entered, boxes)) => {
                    let to = self.camera_on_site_boxes(&boxes);
                    self.selected = match boxes.as_slice() {
                        [one] => vec![site_key(&self.site_nodes()[*one].key)],
                        _ => Vec::new(),
                    };
                    let rect = self.site_boxes_rect(&boxes);
                    (to, rect.map(|rect| (entered, rect)))
                }
                None => (saved.unwrap_or_else(|| self.default_camera()), None),
            },
            // Out to Site: centred on the box holding where this was.
            (Level::Plan(focus), Level::Site) => {
                let anchor = self.site_box_of(focus);
                if let Some(anchor) = anchor {
                    self.selected = vec![site_key(&self.site_nodes()[anchor].key)];
                }
                let to = anchor
                    .map(|ix| self.camera_on_site_box(ix))
                    .or(saved)
                    .unwrap_or_else(|| self.default_camera());
                let rect = anchor.map(|ix| self.site_nodes()[ix].rect);
                (to, old_content.zip(rect))
            }
            // Into a Plan from a box: it grows into the Plan.
            _ => {
                if let Some(focus) = self.plan.focus {
                    self.selected = vec![self.plan.nodes[focus].key.clone()];
                }
                let to = saved.unwrap_or_else(|| self.default_camera());
                let rect = self.content_rect();
                (to, entered.zip(rect))
            }
        };
        self.set_camera(to);
        self.transition = Some(Transition::cross_fade(
            old_scene,
            old_camera,
            to,
            matched,
            self.canvas_size(),
        ));
        self.firing.clear();
        self.after_moving(cx);
        // Domain and Site are one zoom: the gesture carries on through.
        if continuous {
            self.zoom_lock = None;
            if let Some(transition) = self.transition.as_mut() {
                transition.carry_on();
            }
        }
    }

    /// What every move between views does after: the server asked, the
    /// logpoints moved, the view kept.
    pub(super) fn after_moving(&mut self, cx: &mut Context<Self>) {
        self.drag = None;
        self.hover = None;
        self.zoom_lock = Some(Instant::now());
        self.on_stack = self.stack_functions();
        self.ask_server(cx);
        self.save_view();
        cx.emit(MapEvent::LiveLines);
        cx.notify();
    }

    /// The box of the level about to show that's being entered, in the
    /// old level's drawing: a Site box for a module's Plan, a Plan box
    /// for a function's.
    pub(super) fn entered_rect(&self, level: &Level) -> Option<Rect> {
        let Level::Plan(focus) = level else {
            return None;
        };
        match &self.level {
            Level::Site => self.site_box_of(focus).map(|ix| self.site_nodes()[ix].rect),
            Level::Plan(_) | Level::Domain => None,
        }
    }

    /// The Site box holding what a Plan is about.
    pub(super) fn site_box_of(&self, focus: &Focus) -> Option<usize> {
        let site = self.site.as_ref()?;
        match focus {
            Focus::Module(place) => site.box_of(place),
            Focus::Function(symbol) => {
                let place = match &symbol.owner {
                    Some(owner) => format!("{}#{owner}", symbol.file),
                    None => symbol.file.clone(),
                };
                site.box_of(&place)
            }
        }
    }

    /// The old level's whole drawing, or the box in it that matters.
    pub(super) fn content_rect(&self) -> Option<Rect> {
        match &self.level {
            Level::Plan(_) => self.placed.bounds(),
            Level::Site => self.site_nodes().first().map(|root| root.rect),
            Level::Domain => self.domain_level().and_then(|level| level.bounds()),
        }
    }

    /// Down from Plan box `ix`: a function re-centred on, or from the
    /// Plan's own function its code; a "+N more" box opened.
    pub fn enter_plan_box(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(node) = self.plan.nodes.get(ix).cloned() else {
            return;
        };
        match node.item {
            Item::Function(id) if self.plan.focus == Some(ix) => {
                let function = &project.functions[id];
                let path = self.root.join(&project.files[function.file].path);
                cx.emit(MapEvent::OpenCode(path, function.outline.function.name_at));
            }
            Item::Function(id) => {
                self.show(Level::Plan(Focus::Function(project.symbol(id))), cx);
            }
            Item::More { key, .. } => {
                self.opened.more.insert(key);
                self.rebuild(true);
                self.save_view();
                cx.emit(MapEvent::LiveLines);
                cx.notify();
            }
            // A file as a whole: the Plan's own into its code, another to
            // its Plan.
            Item::File { path, .. } if self.plan.focus == Some(ix) => {
                cx.emit(MapEvent::OpenFile(self.root.join(path)));
            }
            Item::File { path, .. } => self.show(Level::Plan(Focus::Module(path)), cx),
            Item::External(_) | Item::Gone(_) => {}
        }
    }

    /// Show box `ix`'s callers, callees or implementations too.
    pub fn expand(&mut self, ix: usize, what: Expand, cx: &mut Context<Self>) {
        let Some(node) = self.plan.nodes.get(ix) else {
            return;
        };
        let key = node.key.clone();
        let set = match what {
            Expand::Callers => &mut self.opened.callers,
            Expand::Callees => &mut self.opened.callees,
            Expand::Implementations => &mut self.opened.implementations,
        };
        set.insert(key);
        self.rebuild(true);
        self.save_view();
        cx.emit(MapEvent::LiveLines);
        cx.notify();
    }
}
