//! The mouse and keys on the Map. Click selects, Shift-click or a box
//! several; dragging moves a Plan's boxes (and only moves them), right or
//! middle drag pans, double-click enters a box. At Domain, dropping a
//! domain onto another merges them; at Site, with the domains' tints
//! showing, a file dragged into another domain's region moves to it. Double-clicking a
//! domain's name renames it. Resting on a Plan's function peeks at its
//! code; on a Site arrow, lists its calls. With a sketching tool picked
//! (B, A, N, F), a press draws instead (`view::sketch`), and a sketch's
//! shapes are selected, moved and edited like boxes. Keys: ⌘↑ out, ⌘↓ or
//! Enter in, ⌘A all, ⌘Z and ⇧⌘Z undo and redo edits to sketches and
//! domains, ⌫ deletes the sketch shapes selected, F2 renames the
//! domain selected, ⌘↩ keeps a proposal, ←/→ step through the agent's
//! trace, Tab or Enter keeps the agent's descriptions, Esc ends a tour,
//! rejects descriptions, clears the selection (or discards a proposal) and
//! then goes back to the code.

use std::time::Instant;

use gpui_kit::*;

use super::super::domains::level::RegionKind;
use super::super::layout::Rect;
use super::super::plan::Item;
use super::domain::Field;
use super::{Level, MapEvent, MapView, site_key};

/// How far, in screen points, the pointer moves before a press drags.
const DRAG_START: f32 = 4.;
/// How long the pointer rests on a function before its code shows.
pub(super) const PEEK_MS: u128 = 500;
/// Lines of a function's code a peek shows.
const PEEK_LINES: usize = 14;
/// How near an arrow, in screen points, the pointer is on it.
const ARROW_REACH: f32 = 6.;

pub(super) enum Drag {
    /// Pressed on box `key`, not moved far yet.
    Press {
        key: String,
        start: Point<Pixels>,
        shift: bool,
    },
    /// Moving the selected Plan boxes from where each was, the pointer
    /// having started at `start` in the drawing.
    Nodes {
        start: (f32, f32),
        from: Vec<(String, (f32, f32))>,
    },
    Marquee {
        from: Point<Pixels>,
        to: Point<Pixels>,
        kept: Vec<String>,
    },
    Pan {
        last: Point<Pixels>,
        moved: bool,
    },
    /// A file or type carried to another domain.
    Member {
        place: String,
        title: String,
        at: Point<Pixels>,
    },
    /// A domain, by its region, carried onto another to merge.
    Region {
        region: usize,
        at: Point<Pixels>,
    },
    /// Drawing with a tool, or on a sketch's shape.
    Sketch(super::sketch::SketchDrag),
}

/// What the pointer is over.
pub(super) enum Hover {
    /// A box, by key, since when.
    Card(String, Instant),
    /// A Site arrow, by its place in the scene drawn, and where.
    Arrow(usize, Point<Pixels>),
}

impl MapView {
    /// The Plan box under `position`.
    pub(super) fn plan_box_at(&self, position: Point<Pixels>) -> Option<usize> {
        let at = self.to_drawing(position);
        (0..self.placed.nodes.len())
            .rev()
            .find(|&ix| self.placed.nodes[ix].contains(at))
    }

    /// The innermost Site box showing under `position`: a box, else the
    /// open frame it's in.
    pub(super) fn site_box_at(&self, position: Point<Pixels>) -> Option<usize> {
        let at = self.to_drawing(position);
        let drawn = self.drawn.borrow();
        drawn
            .card_at(at)
            .map(|card| card.ix)
            .or_else(|| drawn.frame_at(at).map(|frame| frame.ix))
            .or_else(|| {
                // Not drawn yet: the innermost box there.
                let site = self.site.as_ref()?;
                (0..site.nodes.len())
                    .rev()
                    .find(|&ix| site.nodes[ix].rect.contains(at))
            })
    }

    /// The key a box under `position` is selected by.
    fn key_at(&self, position: Point<Pixels>) -> Option<String> {
        match self.level {
            Level::Plan(_) => self
                .plan_box_at(position)
                .map(|ix| self.plan.nodes[ix].key.clone()),
            Level::Site => self
                .site_box_at(position)
                .filter(|&ix| ix != 0)
                .map(|ix| site_key(&self.site_nodes()[ix].key)),
            Level::Domain => self.domain_key_at(position),
        }
    }

    pub(super) fn selected_plan(&self) -> Vec<usize> {
        let mut found: Vec<usize> = self
            .selected
            .iter()
            .filter_map(|key| self.plan.index_of(key))
            .collect();
        found.sort();
        found
    }

    fn select(&mut self, key: String, add: bool) {
        if add {
            match self.selected.iter().position(|k| *k == key) {
                Some(at) => {
                    self.selected.remove(at);
                }
                None => self.selected.push(key),
            }
        } else {
            self.selected = vec![key];
        }
    }

    /// Into the box with key `key`, double-clicked.
    fn enter_key(&mut self, key: &str, cx: &mut Context<Self>) {
        if key.starts_with("domain:") {
            if let Some(ix) = self
                .domain_level()
                .and_then(|level| level.regions.iter().position(|r| r.key == key))
            {
                self.enter_region(ix, cx);
            }
            return;
        }
        match key.strip_prefix("site:") {
            Some(key) => {
                if let Some(ix) = self.site.as_ref().and_then(|site| site.index_of(key)) {
                    self.enter_site_box(ix, cx);
                }
            }
            None => {
                if let Some(ix) = self.plan.index_of(key) {
                    self.enter_plan_box(ix, cx);
                }
            }
        }
    }

    pub(super) fn mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        self.transition = None;
        let position = event.position;
        if self.domains.editing.is_some() {
            self.finish_edit(true, cx);
        }
        if self.sketch_mouse_down(event, window, cx) {
            cx.notify();
            return;
        }
        match self.key_at(position) {
            Some(key) if event.click_count >= 2 && self.on_region_name(&key, position) => {
                if let Some(domain) = self
                    .region_by_key(&key)
                    .and_then(|ix| self.domain_of_region(ix))
                {
                    self.begin_edit(domain, Field::Name, window, cx);
                }
            }
            Some(key) if event.click_count >= 2 => self.enter_key(&key, cx),
            Some(key) => {
                self.drag = Some(Drag::Press {
                    key,
                    start: position,
                    shift: event.modifiers.shift,
                })
            }
            None => {
                let kept = if event.modifiers.shift {
                    self.selected.clone()
                } else {
                    self.selected.clear();
                    Vec::new()
                };
                self.drag = Some(Drag::Marquee {
                    from: position,
                    to: position,
                    kept,
                });
            }
        }
        cx.notify();
    }

    pub(super) fn other_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        self.transition = None;
        self.drag = Some(Drag::Pan {
            last: event.position,
            moved: false,
        });
    }

    pub(super) fn mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        if event.pressed_button.is_none() {
            return;
        }
        let position = event.position;
        let plan = matches!(self.level, Level::Plan(_));
        let carried = match &drag {
            Drag::Press { key, start, .. } if far(position, *start) => self.carry(key, position),
            _ => None,
        };
        self.drag = Some(match drag {
            _ if carried.is_some() => carried.expect("checked"),
            Drag::Sketch(drag) => Drag::Sketch(self.sketch_mouse_move(drag, position)),
            // Site's boxes stay where the project puts them: a drag pans.
            Drag::Press { start, .. } if far(position, start) && !plan => {
                self.offset += position - start;
                Drag::Pan {
                    last: position,
                    moved: true,
                }
            }
            Drag::Press { key, start, .. } if far(position, start) => {
                if !self.selected.contains(&key) {
                    self.selected = vec![key];
                }
                self.glide = None;
                let from: Vec<(String, (f32, f32))> = self
                    .selected
                    .iter()
                    .filter_map(|key| {
                        let rect = self.placed.nodes[self.plan.index_of(key)?];
                        Some((key.clone(), (rect.x, rect.y)))
                    })
                    .collect();
                let start = self.to_drawing(start);
                self.move_nodes(start, &from, position);
                Drag::Nodes { start, from }
            }
            Drag::Nodes { start, from } => {
                self.move_nodes(start, &from, position);
                Drag::Nodes { start, from }
            }
            Drag::Marquee { from, kept, .. } => {
                let (x0, y0) = self.to_drawing(from);
                let (x1, y1) = self.to_drawing(position);
                let area = Rect {
                    x: x0.min(x1),
                    y: y0.min(y1),
                    width: (x0 - x1).abs(),
                    height: (y0 - y1).abs(),
                };
                let mut selected = kept.clone();
                let touched: Vec<String> = match self.level {
                    Level::Plan(_) => self
                        .placed
                        .nodes
                        .iter()
                        .enumerate()
                        .filter(|(_, rect)| rect.overlaps(&area))
                        .map(|(ix, _)| self.plan.nodes[ix].key.clone())
                        .collect(),
                    Level::Site => self
                        .drawn
                        .borrow()
                        .cards
                        .iter()
                        .filter(|card| card.rect.overlaps(&area))
                        .map(|card| site_key(&card.key))
                        .collect(),
                    Level::Domain => self
                        .drawn
                        .borrow()
                        .cards
                        .iter()
                        .filter(|card| card.rect.overlaps(&area))
                        .map(|card| card.key.clone())
                        .collect(),
                };
                for key in touched {
                    if !selected.contains(&key) {
                        selected.push(key);
                    }
                }
                self.selected = selected;
                Drag::Marquee {
                    from,
                    to: position,
                    kept,
                }
            }
            Drag::Pan { last, .. } => {
                self.offset += position - last;
                Drag::Pan {
                    last: position,
                    moved: true,
                }
            }
            Drag::Member { place, title, .. } => Drag::Member {
                place,
                title,
                at: position,
            },
            Drag::Region { region, .. } => Drag::Region {
                region,
                at: position,
            },
            other => other,
        });
        cx.notify();
    }

    fn move_nodes(
        &mut self,
        start: (f32, f32),
        from: &[(String, (f32, f32))],
        position: Point<Pixels>,
    ) {
        let (x, y) = self.to_drawing(position);
        let (dx, dy) = (x - start.0, y - start.1);
        for (key, (fx, fy)) in from {
            let place = (fx + dx, fy + dy);
            self.places.insert(key.clone(), place);
            if let Some(ix) = self.plan.index_of(key) {
                self.placed.put(ix, place);
            }
        }
        self.placed.refit(&self.plan);
    }

    pub(super) fn mouse_up(
        &mut self,
        _: &MouseUpEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        match drag {
            Drag::Sketch(drag) => self.sketch_mouse_up(drag, window, cx),
            Drag::Press { key, shift, .. } => self.select(key, shift),
            Drag::Nodes { .. } => self.save_view(),
            Drag::Marquee { .. } => {}
            Drag::Pan { moved, .. } => {
                if moved {
                    self.save_view();
                }
            }
            Drag::Member { place, at, .. } => {
                if let Some((home, _)) = self.drop_home(at)
                    && home != self.home_now(&place)
                {
                    self.move_to_region(&place, home, cx);
                }
            }
            Drag::Region { region, at } => {
                let into = self.region_at(at).and_then(|ix| self.domain_of_region(ix));
                if let (Some(from), Some(into)) = (self.domain_of_region(region), into)
                    && from != into
                {
                    self.merge_domains(from, into, cx);
                }
            }
        }
        cx.notify();
    }

    /// A press on box `key` moved far: carrying a file or type to another
    /// domain, or a domain onto another, where that's what a drag does.
    fn carry(&self, key: &str, position: Point<Pixels>) -> Option<Drag> {
        let editable = !self.drafts();
        match self.level {
            Level::Domain if editable => {
                let region = self.region_by_key(key)?;
                self.domain_of_region(region)?;
                Some(Drag::Region {
                    region,
                    at: position,
                })
            }
            Level::Site if self.has_tints() && !self.domains.tints_off => {
                let place = self.site_place(key)?;
                let title = place.rsplit('/').next().unwrap_or(&place).to_string();
                Some(Drag::Member {
                    place,
                    title,
                    at: position,
                })
            }
            _ => None,
        }
    }

    /// Region `key`'s place in the Domain level.
    pub(super) fn region_by_key(&self, key: &str) -> Option<usize> {
        self.domain_level()?
            .regions
            .iter()
            .position(|region| region.key == key)
    }

    /// Whether `position` is on the name of domain region `key`, drawn
    /// above it.
    fn on_region_name(&self, key: &str, position: Point<Pixels>) -> bool {
        let Some(level) = self.domain_level() else {
            return false;
        };
        let Some(region) = level.regions.iter().find(|region| region.key == key) else {
            return false;
        };
        if !matches!(region.kind, RegionKind::Domain(_)) {
            return false;
        }
        self.label_rect(region).contains(self.to_drawing(position))
    }

    /// The pointer moved, no button down: what it's over lights, and a
    /// peek or a list of calls follows if it rests there.
    pub(super) fn hover_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = event.position;
        let card = match self.level {
            Level::Plan(_) => self
                .plan_box_at(position)
                .map(|ix| self.plan.nodes[ix].key.clone()),
            Level::Site | Level::Domain => None,
        };
        let hover = match card {
            Some(key) => match &self.hover {
                Some(Hover::Card(was, since)) if *was == key => Some(Hover::Card(key, *since)),
                _ => {
                    // Shown once it has rested long enough.
                    cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(PEEK_MS as u64 + 20))
                            .await;
                        this.update(cx, |_, cx| cx.notify()).ok();
                    })
                    .detach();
                    Some(Hover::Card(key, Instant::now()))
                }
            },
            None => self.arrow_at(position).map(|ix| Hover::Arrow(ix, position)),
        };
        let changed =
            !matches!((&hover, &self.hover), (None, None)) && !same_hover(&hover, &self.hover);
        self.hover = hover;
        if changed {
            cx.notify();
        }
    }

    /// The Site arrow nearest `position`, if one is near enough.
    fn arrow_at(&self, position: Point<Pixels>) -> Option<usize> {
        let at = self.to_drawing(position);
        let reach = ARROW_REACH / self.zoom;
        let drawn = self.drawn.borrow();
        drawn
            .arrows
            .iter()
            .enumerate()
            .filter(|(_, arrow)| !arrow.across)
            .map(|(ix, arrow)| (ix, distance_to_bow(arrow.from, arrow.to, at)))
            .filter(|(_, d)| *d < reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(ix, _)| ix)
    }

    /// The first lines of the code of Plan box `key`'s function.
    pub(super) fn peek(&self, key: &str) -> Option<Vec<String>> {
        let project = self.project.as_ref()?;
        let ix = self.plan.index_of(key)?;
        let Item::Function(id) = self.plan.nodes[ix].item else {
            return None;
        };
        let function = &project.functions[id];
        let path = self.root.join(&project.files[function.file].path);
        let text = std::fs::read_to_string(path).ok()?;
        let code = text.get(function.range().clone())?;
        let mut lines: Vec<String> = code
            .lines()
            .take(PEEK_LINES)
            .map(|line| line.replace('\t', "    "))
            .collect();
        if code.lines().count() > PEEK_LINES {
            lines.push("…".into());
        }
        // Lined up as written, the first line's indent dropped.
        let indent = code
            .lines()
            .nth(1)
            .map_or(0, |line| line.len() - line.trim_start().len())
            .saturating_sub(4);
        Some(
            lines
                .into_iter()
                .enumerate()
                .map(|(n, line)| {
                    if n == 0 {
                        line
                    } else {
                        line.chars()
                            .skip(indent.min(line.len() - line.trim_start().len()))
                            .collect()
                    }
                })
                .collect(),
        )
    }

    pub(super) fn scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(16.));
        if event.modifiers.secondary() || event.modifiers.control {
            self.zoom_at(1. - f32::from(delta.y) / 300., event.position, cx);
        } else {
            self.transition = None;
            self.offset += delta;
            self.moved(cx);
            cx.notify();
        }
    }

    pub(super) fn pinch(&mut self, event: &PinchEvent, _: &mut Window, cx: &mut Context<Self>) {
        if event.phase == TouchPhase::Ended {
            self.zoom_lock = None;
            return;
        }
        self.zoom_at(1. + event.delta, event.position, cx);
    }

    pub(super) fn key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Keys typed in a field on the Map are the field's.
        if !self.focus.is_focused(window) {
            return;
        }
        let keystroke = &event.keystroke;
        let secondary = keystroke.modifiers.secondary();
        match keystroke.key.as_str() {
            "escape" => {
                if self.drag.take().is_some() {
                } else if self.sketches.tool != super::sketch::Tool::Pointer {
                    self.set_tool(super::sketch::Tool::Pointer, cx);
                } else if self.sketches.list_open {
                    self.sketches.list_open = false;
                } else if self.domains.editing.is_some() {
                    self.finish_edit(false, cx);
                } else if self.end_tour(cx) {
                } else if let Some(source) = self.descriptions_waiting() {
                    self.settle_descriptions(source, false, cx);
                } else if !self.selected.is_empty() {
                    self.selected.clear();
                } else if self.level == Level::Domain && self.domains.proposal.is_some() {
                    self.discard_proposal(cx);
                } else {
                    cx.emit(MapEvent::Close);
                }
            }
            "enter" if secondary && self.domains.proposal.is_some() => self.keep_proposal(cx),
            "z" if secondary && keystroke.modifiers.shift => {
                if !self.redo_sketches(cx) && !self.redo_domains(cx) {
                    return;
                }
            }
            "z" if secondary => {
                if !self.undo_sketches(cx) && !self.undo_domains(cx) {
                    return;
                }
            }
            "backspace" | "delete" if !secondary => {
                if !self.delete_selected_shapes(cx) {
                    return;
                }
            }
            "b" | "a" | "n" | "f"
                if !secondary
                    && !keystroke.modifiers.shift
                    && !keystroke.modifiers.alt
                    && !keystroke.modifiers.control =>
            {
                if !self.tool_key(&keystroke.key, cx) {
                    return;
                }
            }
            "f2" => {
                let domain = self
                    .selected
                    .iter()
                    .find_map(|key| self.region_by_key(key))
                    .and_then(|ix| self.domain_of_region(ix));
                match domain {
                    Some(domain) => self.begin_edit(domain, Field::Name, window, cx),
                    None => return,
                }
            }
            "enter" if !secondary && self.descriptions_waiting().is_some() => {
                if let Some(source) = self.descriptions_waiting() {
                    self.settle_descriptions(source, true, cx);
                }
            }
            "left" | "right" if !secondary && !keystroke.modifiers.shift => {
                let delta = if keystroke.key == "left" { -1 } else { 1 };
                if !self.tour_by(delta, cx) {
                    return;
                }
            }
            "up" if secondary => self.step_out(cx),
            "down" if secondary => self.step_in(cx),
            "enter" => self.step_in(cx),
            "a" if secondary => {
                self.selected = match self.level {
                    Level::Plan(_) => self.plan.nodes.iter().map(|n| n.key.clone()).collect(),
                    Level::Site => self
                        .drawn
                        .borrow()
                        .cards
                        .iter()
                        .map(|card| site_key(&card.key))
                        .collect(),
                    Level::Domain => self
                        .drawn
                        .borrow()
                        .regions
                        .iter()
                        .map(|region| region.key.clone())
                        .collect(),
                };
            }
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }
}

fn same_hover(a: &Option<Hover>, b: &Option<Hover>) -> bool {
    match (a, b) {
        (Some(Hover::Card(a, _)), Some(Hover::Card(b, _))) => a == b,
        (Some(Hover::Arrow(a, _)), Some(Hover::Arrow(b, _))) => a == b,
        (None, None) => true,
        _ => false,
    }
}

pub(super) fn far(a: Point<Pixels>, b: Point<Pixels>) -> bool {
    far_by(a, b, DRAG_START)
}

/// Whether `a` and `b` are more than `by` screen points apart.
pub(super) fn far_by(a: Point<Pixels>, b: Point<Pixels>, by: f32) -> bool {
    let d = a - b;
    f32::from(d.x).hypot(f32::from(d.y)) > by
}

/// How far `at` is from the bowed arrow from `from` to `to`, as drawn.
pub(super) fn distance_to_bow(from: (f32, f32), to: (f32, f32), at: (f32, f32)) -> f32 {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let bend = 0.12;
    let control = (from.0 + dx / 2. + dy * bend, from.1 + dy / 2. - dx * bend);
    (0..=16)
        .map(|step| {
            let t = step as f32 / 16.;
            let u = 1. - t;
            let x = from.0 * u * u + control.0 * 2. * u * t + to.0 * t * t;
            let y = from.1 * u * u + control.1 * 2. * u * t + to.1 * t * t;
            (x - at.0).hypot(y - at.1)
        })
        .fold(f32::MAX, f32::min)
}
