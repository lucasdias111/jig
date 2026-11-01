//! Pressing, dragging and letting go: selecting, moving boxes, carrying a file or domain.

use super::*;

impl MapView {
    pub(in crate::ui::view) fn mouse_down(
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

    pub(in crate::ui::view) fn other_down(
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

    pub(in crate::ui::view) fn mouse_move(
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

    pub(super) fn move_nodes(
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

    pub(in crate::ui::view) fn mouse_up(
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
    pub(super) fn carry(&self, key: &str, position: Point<Pixels>) -> Option<Drag> {
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

    /// Whether `position` is on the name of domain region `key`, drawn
    /// above it.
    pub(super) fn on_region_name(&self, key: &str, position: Point<Pixels>) -> bool {
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
}
