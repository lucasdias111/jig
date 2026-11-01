//! The Map's keys.

use super::*;

impl MapView {
    pub(in crate::ui::view) fn key_down(
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
                } else if self.sketches.tool != super::super::sketch::Tool::Pointer {
                    self.set_tool(super::super::sketch::Tool::Pointer, cx);
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

    /// Into the box with key `key`, double-clicked.
    pub(super) fn enter_key(&mut self, key: &str, cx: &mut Context<Self>) {
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
}
