//! The agent's drawings on the Map (`map::lens`): those of the
//! conversation on screen, and the pinned ones on a layer of their own.
//! Every place a drawing names lands on the box holding it at the level
//! showing: its function at Plan, else the box it's in at Site as far as
//! it's open, else its domain. A trace's steps fold onto those boxes, so
//! at a coarse level several steps make one stop, listing their range.
//! Drawings never move the camera: a tour does, step by step with ←/→,
//! across levels as it needs, and so does a link to a place.

use std::cell::RefCell;
use std::collections::HashMap;

use gpui_kit::*;

use super::super::layout::Rect;
use super::super::lens::pins::{self, Pin};
use super::super::lens::{Drawing, Place, purposes, resolve};
use super::super::plan::{Focus, Item, function_key};
use super::zoom::Transition;
use super::{Level, MapEvent, MapView, site_key};

/// Which drawing: the conversation's, by its id there, or a pin's, by its
/// place in the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Conversation(usize),
    Pin(usize),
}

/// A tour under way: the drawing, and the step showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tour {
    pub source: Source,
    pub step: usize,
}

/// What the Map keeps of the agent's drawings.
pub struct LensState {
    /// The conversation's drawings showing, by their id in it.
    pub drawings: Vec<(usize, Drawing)>,
    pub pins: Vec<Pin>,
    /// The Pinned layer shows.
    pub pins_shown: bool,
    pub tour: Option<Tour>,
    /// Where the conversation on screen was last drawn, in the window: the
    /// list of drawings keeps clear of it.
    pub conversation: Option<jig_commands::chat::ChatBounds>,
    /// Places as resolved against the project, by name: forgotten when
    /// it's read again.
    resolved: RefCell<HashMap<String, Option<Place>>>,
}

impl Default for LensState {
    fn default() -> Self {
        Self {
            drawings: Vec::new(),
            pins: Vec::new(),
            pins_shown: true,
            tour: None,
            conversation: None,
            resolved: RefCell::default(),
        }
    }
}

impl LensState {
    pub fn forget_places(&self) {
        self.resolved.borrow_mut().clear();
    }
}

/// A box a place lands on at the level showing: its key, and where it is
/// in the drawing.
#[derive(Clone, Debug, PartialEq)]
pub struct Landing {
    pub key: String,
    pub rect: Rect,
}

/// Where a trace stops at the level showing: a box, and the steps that
/// land on it in a row.
#[derive(Clone, Debug, PartialEq)]
pub struct Stop {
    pub landing: Landing,
    pub steps: Vec<usize>,
}

impl Stop {
    /// Its steps as people count them: `3`, `3–5`.
    pub fn range(&self) -> String {
        match (self.steps.first(), self.steps.last()) {
            (Some(first), Some(last)) if first != last => format!("{}–{}", first + 1, last + 1),
            (Some(first), _) => (first + 1).to_string(),
            _ => String::new(),
        }
    }
}

/// `landings` (one per step, `None` where the level has no box for it)
/// folded: steps in a row on the same box make one stop.
pub fn fold(landings: &[Option<Landing>]) -> Vec<Stop> {
    let mut stops: Vec<Stop> = Vec::new();
    for (step, landing) in landings.iter().enumerate() {
        let Some(landing) = landing else {
            continue;
        };
        match stops.last_mut() {
            Some(stop) if stop.landing.key == landing.key => stop.steps.push(step),
            _ => stops.push(Stop {
                landing: landing.clone(),
                steps: vec![step],
            }),
        }
    }
    stops
}

impl MapView {
    /// The conversation's drawings to show, by their ids in it; none when
    /// it's closed.
    pub fn set_drawings(&mut self, drawings: Vec<(usize, Drawing)>, cx: &mut Context<Self>) {
        if drawings == self.lens.drawings {
            return;
        }
        if let Some(Tour {
            source: Source::Conversation(id),
            ..
        }) = self.lens.tour
            && !drawings.iter().any(|(at, _)| *at == id)
        {
            self.lens.tour = None;
        }
        self.lens.drawings = drawings;
        cx.notify();
    }

    /// Where the conversation is drawn, for the list of drawings to keep
    /// clear of.
    pub fn set_conversation_bounds(&mut self, bounds: Option<jig_commands::chat::ChatBounds>) {
        self.lens.conversation = bounds;
    }

    /// The pins as saved now.
    pub(super) fn reload_pins(&mut self) {
        self.lens.pins = pins::read_all(&self.root);
        if matches!(
            self.lens.tour,
            Some(Tour {
                source: Source::Pin(_),
                ..
            })
        ) {
            self.lens.tour = None;
        }
    }

    /// The Pinned layer shown or not.
    pub fn set_pins_shown(&mut self, on: bool, cx: &mut Context<Self>) {
        self.lens.pins_shown = on;
        cx.notify();
    }

    /// Pin `drawing`: saved in `.jig/map/pins/` and shown on the Pinned
    /// layer. Its pin's name back.
    pub fn pin(&mut self, drawing: &Drawing, cx: &mut Context<Self>) -> anyhow::Result<String> {
        let name = pins::save(&self.root, drawing)?;
        self.reload_pins();
        self.lens.pins_shown = true;
        cx.notify();
        Ok(name)
    }

    /// Unpin the pin named `name`.
    pub fn unpin(&mut self, name: &str, cx: &mut Context<Self>) {
        if let Err(error) = pins::remove(&self.root, name) {
            self.say(&format!("{error:#}"), cx);
        }
        self.reload_pins();
        cx.notify();
    }

    /// Every drawing showing: the conversation's, then the pins'.
    pub(super) fn drawings_showing(&self) -> Vec<(Source, &Drawing)> {
        let mut showing: Vec<(Source, &Drawing)> = self
            .lens
            .drawings
            .iter()
            .map(|(id, drawing)| (Source::Conversation(*id), drawing))
            .collect();
        if self.lens.pins_shown {
            showing.extend(
                self.lens
                    .pins
                    .iter()
                    .enumerate()
                    .map(|(ix, pin)| (Source::Pin(ix), &pin.drawing)),
            );
        }
        showing
    }

    pub(super) fn drawing(&self, source: Source) -> Option<&Drawing> {
        match source {
            Source::Conversation(id) => self
                .lens
                .drawings
                .iter()
                .find(|(at, _)| *at == id)
                .map(|(_, d)| d),
            Source::Pin(ix) => self.lens.pins.get(ix).map(|pin| &pin.drawing),
        }
    }

    /// Whether every place `drawing` names is in the project: as the tool
    /// checked it, so a drawing it turned down isn't drawn. Before the
    /// project is read, nothing can be told.
    pub fn draws(&self, drawing: &Drawing) -> bool {
        self.project.is_none()
            || drawing
                .places()
                .iter()
                .all(|place| self.place(place).is_some())
    }

    /// The place named `name`, as the project has it.
    pub(super) fn place(&self, name: &str) -> Option<Place> {
        if let Some(found) = self.lens.resolved.borrow().get(name) {
            return found.clone();
        }
        let project = self.project.as_ref()?;
        let domains = self.shown_domains();
        let found = resolve(project, domains.as_ref(), name).ok();
        self.lens
            .resolved
            .borrow_mut()
            .insert(name.to_string(), found.clone());
        found
    }

    /// The box place `name` lands on at the level showing, if it has one.
    pub(super) fn landing(&self, name: &str) -> Option<Landing> {
        let place = self.place(name)?;
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
    pub(super) fn site_box_of_place(&self, place: &Place) -> Option<usize> {
        let site = self.site.as_ref()?;
        match place {
            Place::Domain(name) => site.index_of(&format!("domain:{name}")),
            place => site.box_of(&place.site_place()?),
        }
    }

    /// At Plan: the function's box, else the frame of the type or file,
    /// else the file's box.
    fn plan_landing(&self, place: &Place) -> Option<Landing> {
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

    fn fly_to_step(&mut self, cx: &mut Context<Self>) {
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
    pub(super) fn pan_to(&mut self, rect: Rect, cx: &mut Context<Self>) {
        let from = self.camera();
        let (x, y) = rect.centre();
        let to = super::zoom::Camera { x, y, ..from };
        self.transition = Some(Transition::pan(from, to));
        self.set_camera(to);
        self.after_moving(cx);
    }

    /// Pins whose places are gone: by name, with how many are gone.
    pub fn orphaned_pins(&self) -> Vec<(String, usize)> {
        if self.project.is_none() {
            return Vec::new();
        }
        self.lens
            .pins
            .iter()
            .filter_map(|pin| {
                let gone = pin
                    .drawing
                    .places()
                    .into_iter()
                    .filter(|place| self.place(place).is_none())
                    .count();
                (gone > 0).then(|| (pin.name.clone(), gone))
            })
            .collect()
    }

    /// The conversation's descriptions waiting to be kept or not.
    pub(super) fn descriptions_waiting(&self) -> Option<Source> {
        self.lens
            .drawings
            .iter()
            .find(|(_, drawing)| drawing.kind == super::super::lens::Kind::Describe)
            .map(|(id, _)| Source::Conversation(*id))
    }

    /// The agent's descriptions in drawing `source` kept or not: kept, the
    /// purposes go where the Map keeps them and the drawing goes either
    /// way.
    pub fn settle_descriptions(&mut self, source: Source, keep: bool, cx: &mut Context<Self>) {
        let Some(drawing) = self.drawing(source).cloned() else {
            return;
        };
        if keep {
            self.keep_descriptions(&drawing, cx);
        }
        if let Source::Conversation(id) = source {
            cx.emit(MapEvent::Settled(id, keep));
        }
        cx.notify();
    }

    /// Keep the purposes and labels `drawing` describes: a domain's in
    /// `domains.toml`, with the label of a link between two domains; the
    /// rest in `.jig/map/purposes.toml`.
    fn keep_descriptions(&mut self, drawing: &Drawing, cx: &mut Context<Self>) {
        let mut kept = purposes::read(&self.root);
        let mut domain_purposes = Vec::new();
        let mut flows = Vec::new();
        for step in &drawing.steps {
            match self.place(&step.place) {
                Some(Place::Domain(name)) => domain_purposes.push((name, step.label.clone())),
                Some(place) => {
                    if let Some(at) = place.site_place() {
                        kept.purposes.insert(at, step.label.clone());
                    }
                }
                None => {}
            }
        }
        for link in &drawing.links {
            match (self.place(&link.from), self.place(&link.to)) {
                (Some(Place::Domain(from)), Some(Place::Domain(to))) => {
                    flows.push((from, to, link.label.clone()))
                }
                (Some(from), Some(to)) => {
                    let name = |place: &Place| match place {
                        Place::Domain(name) => format!("domain:{name}"),
                        place => place.site_place().unwrap_or_default(),
                    };
                    kept.set_label(&name(&from), &name(&to), &link.label);
                }
                _ => {}
            }
        }
        if let Err(error) = purposes::write(&self.root, &kept) {
            self.say(&format!("{error:#}"), cx);
        }
        if !domain_purposes.is_empty() || !flows.is_empty() {
            self.edit_domains(
                |domains| {
                    for (name, purpose) in &domain_purposes {
                        if let Some(ix) = domains.index_of(name) {
                            domains.set_purpose(ix, purpose);
                        }
                    }
                    for (from, to, label) in &flows {
                        domains.set_flow(from, to, label);
                    }
                },
                cx,
            );
        } else {
            self.domains_changed();
        }
    }
}

#[cfg(test)]
impl MapView {
    /// The boxes the agent's drawings land on at the level showing, by
    /// key, with their badges and words.
    pub fn lens_marks(&self) -> Vec<(String, Vec<String>, Vec<String>)> {
        self.lens_picture(&|rect| rect)
            .boxes
            .into_iter()
            .map(|marked| (marked.key, marked.badges, marked.words))
            .collect()
    }

    /// The tour's step, if one is under way.
    pub fn tour_step(&self) -> Option<usize> {
        self.lens.tour.map(|tour| tour.step)
    }

    /// Touring the conversation's drawing `id` at `step`, the camera left
    /// where it is: for pictures of a tour.
    pub fn show_tour_step(&mut self, id: usize, step: usize) {
        self.lens.tour = Some(Tour {
            source: Source::Conversation(id),
            step,
        });
    }

    /// Where the canvas is in the window.
    pub fn canvas_bounds(&self) -> Bounds<Pixels> {
        self.bounds.get()
    }

    pub fn pin_names(&self) -> Vec<String> {
        self.lens.pins.iter().map(|pin| pin.name.clone()).collect()
    }

    pub fn drawings_shown(&self) -> usize {
        self.drawings_showing().len()
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::layout::Rect;
    use super::{Landing, fold};

    fn landing(key: &str) -> Option<Landing> {
        Some(Landing {
            key: key.into(),
            rect: Rect::default(),
        })
    }

    #[test]
    fn steps_in_a_row_on_one_box_fold_into_one_stop() {
        let stops = fold(&[landing("a"), landing("a"), None, landing("b"), landing("a")]);
        let ranges: Vec<(String, String)> = stops
            .iter()
            .map(|stop| (stop.landing.key.clone(), stop.range()))
            .collect();
        assert_eq!(
            ranges,
            [
                ("a".into(), "1–2".into()),
                ("b".into(), "4".into()),
                ("a".into(), "5".into())
            ]
        );
    }
}
