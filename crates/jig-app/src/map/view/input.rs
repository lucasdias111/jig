//! The mouse and keys on the Map. Click selects, Shift-click or a box
//! several; dragging moves a Plan's boxes (and only moves them), right or
//! middle drag pans, double-click enters a box. Resting on a Plan's
//! function peeks at its code; on a Site arrow, lists its calls. Keys:
//! ⌘↑ out, ⌘↓ or Enter in, ⌘A all, Esc clears the selection and then
//! goes back to the code.

use std::time::Instant;

use gpui_kit::*;

use super::super::layout::Rect;
use super::super::plan::Item;
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
        match self.key_at(position) {
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
        self.drag = Some(match drag {
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

    pub(super) fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        match drag {
            Drag::Press { key, shift, .. } => self.select(key, shift),
            Drag::Nodes { .. } => self.save_view(),
            Drag::Marquee { .. } => {}
            Drag::Pan { moved, .. } => {
                if moved {
                    self.save_view();
                }
            }
        }
        cx.notify();
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
            Level::Site => None,
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
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keystroke = &event.keystroke;
        let secondary = keystroke.modifiers.secondary();
        match keystroke.key.as_str() {
            "escape" => {
                if self.drag.take().is_some() {
                } else if !self.selected.is_empty() {
                    self.selected.clear();
                } else {
                    cx.emit(MapEvent::Close);
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

fn far(a: Point<Pixels>, b: Point<Pixels>) -> bool {
    let d = a - b;
    f32::from(d.x).hypot(f32::from(d.y)) > DRAG_START
}

/// How far `at` is from the bowed arrow from `from` to `to`, as drawn.
fn distance_to_bow(from: (f32, f32), to: (f32, f32), at: (f32, f32)) -> f32 {
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
