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

use super::zoom::Transition;
use super::{Level, MapEvent, MapView, site_key};
use crate::domain::layout::Rect;
use crate::domain::lens::{Drawing, Place, resolve};
use crate::domain::plan::{Focus, Item, function_key};
use crate::store::cache::Folder;
use crate::store::persist::pins::{self, Pin};
use crate::store::persist::purposes;
mod descriptions;
mod landing;
mod touring;

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
}

#[cfg(any(test, feature = "testing"))]
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
mod tests;
