//! Asking the agent from the Map (⌘K there): what it's told besides the
//! question. The boxes selected (names, places, purposes; at Plan, their
//! signatures and where they are; at Domain, their entries in
//! `domains.toml`), the user's sketches with a shape selected, as data
//! with what they're attached to (`view::sketch`), and a compact outline
//! of the level showing: boxes, arrows and counts. No code is sent: the agent reads what it needs.
//! Explain this level asks it to describe what shows.

use gpui_kit::*;

use super::{Level, MapEvent, MapView};
use crate::domain::domains::level::RegionKind;
use crate::domain::plan::{Focus, Item};
use crate::store::cache::Folder;

/// Most boxes and arrows an outline lists.
const MOST_BOXES: usize = 60;
const MOST_ARROWS: usize = 30;
/// Most members of a selected domain listed.
const MOST_MEMBERS: usize = 15;

/// A question asked on the Map: what the agent is told about it, and
/// where the selection is in the window, to open beside.
pub struct Question {
    pub context: String,
    pub beside: Option<Point<Pixels>>,
}

impl MapView {
    /// What the agent is told about the Map as the user sees it now.
    pub fn question(&self) -> Question {
        let mut context = format!(
            "I'm in Jig's Map of this project, at its {} level{}.",
            self.level.name(),
            match &self.level {
                Level::Plan(Focus::Function(symbol)) => {
                    format!(", around {}#{}", symbol.file, symbol.title())
                }
                Level::Plan(Focus::Module(place)) => format!(", of {place}"),
                _ => String::new(),
            }
        );
        let selected = self.selected_lines();
        if !selected.is_empty() {
            context.push_str("\n\nSelected:\n");
            context.push_str(&selected.join("\n"));
        }
        for sketch in self.sketch_lines() {
            context.push_str("\n\n");
            context.push_str(&sketch);
        }
        let outline = self.outline_lines();
        if !outline.is_empty() {
            context.push_str("\n\nWhat the level shows:\n");
            context.push_str(&outline.join("\n"));
        }
        Question {
            context,
            beside: self.selection_beside(),
        }
    }

    /// Explain this level: the agent asked to describe what shows.
    pub fn explain_level(&mut self, cx: &mut Context<Self>) {
        let what = match self.level {
            Level::Domain => "each domain",
            Level::Site => "each box showing",
            Level::Plan(_) => "each function showing",
        };
        cx.emit(MapEvent::Explain(format!(
            "Explain this level of the Map to me. Read the code behind it first. Then propose a one-line purpose for {what} with map_describe (and labels for the arrows that matter most), and show how they work together with a map_trace or map_group where it helps. Sum it up in a few sentences."
        )));
    }

    /// Each box selected, a line each.
    fn selected_lines(&self) -> Vec<String> {
        let project = self.project.as_deref();
        let mut lines = Vec::new();
        for key in &self.selected {
            match &self.level {
                Level::Plan(_) => {
                    let Some(ix) = self.plan.index_of(key) else {
                        continue;
                    };
                    let node = &self.plan.nodes[ix];
                    let line = match (&node.item, project) {
                        (Item::Function(id), Some(project)) => {
                            let function = &project.functions[*id];
                            let path = project.files[function.file].path.clone();
                            let at = line_of(
                                Folder(&self.root).text(&path),
                                function.outline.function.name_at,
                            );
                            format!(
                                "- {}#{} {} ({}:{at})",
                                crate::domain::lens::slashed(&path),
                                function.title(),
                                node.detail,
                                crate::domain::lens::slashed(&path),
                            )
                        }
                        (Item::File { path, .. }, _) => format!("- {path} (a file)"),
                        _ => format!("- {}", node.title),
                    };
                    lines.push(line);
                }
                Level::Site => {
                    let Some(site) = self.site.as_ref() else {
                        continue;
                    };
                    let Some(ix) = site.index_of(key.strip_prefix("site:").unwrap_or(key)) else {
                        continue;
                    };
                    let node = &site.nodes[ix];
                    let place = if node.place.is_empty() {
                        node.key.clone()
                    } else {
                        node.place.clone()
                    };
                    let mut line = format!("- {} ({place}): {}", node.title, node.detail);
                    if let Some(purpose) = &node.purpose {
                        line.push_str(&format!(" — {purpose}"));
                    }
                    lines.push(line);
                }
                Level::Domain => lines.extend(self.domain_entry(key)),
            }
        }
        lines
    }

    /// A domain selected, as `domains.toml` has it.
    fn domain_entry(&self, key: &str) -> Vec<String> {
        let name = key.strip_prefix("domain:").unwrap_or(key);
        let domains = self.shown_domains();
        let Some(domain) = domains
            .as_ref()
            .and_then(|d| d.domains.iter().find(|d| d.name == name))
        else {
            return vec![format!("- {name}")];
        };
        let mut lines = vec![format!("- domain:{}: {}", domain.name, domain.purpose)];
        if !domain.offers.is_empty() {
            lines.push(format!("  offers: {}", domain.offers.join("; ")));
        }
        let members: Vec<&str> = domain
            .members
            .iter()
            .take(MOST_MEMBERS)
            .map(String::as_str)
            .collect();
        if !members.is_empty() {
            let more = domain.members.len().saturating_sub(MOST_MEMBERS);
            lines.push(format!(
                "  members: {}{}",
                members.join(", "),
                if more > 0 {
                    format!(" and {more} more")
                } else {
                    String::new()
                }
            ));
        }
        lines
    }

    /// The level showing in brief: its boxes, then its arrows.
    fn outline_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        match &self.level {
            Level::Domain => {
                let Some(level) = self.domain_level() else {
                    return lines;
                };
                for (_, region) in level.showing() {
                    let name = match region.kind {
                        RegionKind::Domain(_) => format!("domain:{}", region.name),
                        _ => region.name.clone(),
                    };
                    lines.push(format!(
                        "- {name} ({}): {}",
                        region.detail(),
                        region.purpose
                    ));
                }
                for link in &level.links {
                    let (a, b) = (&level.regions[link.a].name, &level.regions[link.b].name);
                    for label in &link.forward {
                        lines.push(format!("- {a} → {b}: {label}"));
                    }
                    for label in &link.backward {
                        lines.push(format!("- {b} → {a}: {label}"));
                    }
                }
            }
            Level::Site => {
                let drawn = self.drawn.borrow();
                let Some(site) = self.site.as_ref() else {
                    return lines;
                };
                let boxes = drawn
                    .frames
                    .iter()
                    .map(|frame| frame.ix)
                    .chain(drawn.cards.iter().map(|card| card.ix));
                for ix in boxes.take(MOST_BOXES) {
                    let node = &site.nodes[ix];
                    if node.place.is_empty() {
                        continue;
                    }
                    lines.push(format!(
                        "- {} ({}): {}",
                        node.title, node.place, node.detail
                    ));
                }
                for arrow in drawn
                    .arrows
                    .iter()
                    .filter(|a| a.calls.is_some())
                    .take(MOST_ARROWS)
                {
                    let place = |key: &str| {
                        site.index_of(key)
                            .map(|ix| site.nodes[ix].place.clone())
                            .unwrap_or_else(|| key.to_string())
                    };
                    let count = arrow.calls.as_ref().map_or(0, |calls| calls.count);
                    lines.push(format!(
                        "- {} → {}: {count} calls",
                        place(&arrow.keys.0),
                        place(&arrow.keys.1)
                    ));
                }
            }
            Level::Plan(_) => {
                let Some(project) = self.project.as_deref() else {
                    return lines;
                };
                let name = |ix: usize| match &self.plan.nodes[ix].item {
                    Item::Function(id) => {
                        let function = &project.functions[*id];
                        format!(
                            "{}#{}",
                            crate::domain::lens::slashed(&project.files[function.file].path),
                            function.title()
                        )
                    }
                    _ => self.plan.nodes[ix].title.clone(),
                };
                for ix in (0..self.plan.nodes.len()).take(MOST_BOXES) {
                    let node = &self.plan.nodes[ix];
                    lines.push(format!("- {} {}", name(ix), node.detail));
                }
                for wire in self.plan.wires.iter().take(MOST_ARROWS) {
                    lines.push(format!("- {} calls {}", name(wire.from), name(wire.to)));
                }
            }
        }
        lines
    }

    /// Just right of the boxes selected, in the window: where the input
    /// and the conversation open, so they cover none of them.
    fn selection_beside(&self) -> Option<Point<Pixels>> {
        let rects: Vec<crate::domain::layout::Rect> = self
            .selected
            .iter()
            .filter_map(|key| match &self.level {
                _ if key.starts_with(crate::domain::sketch::PREFIX) => {
                    self.shape_rect(&crate::domain::sketch::ShapeRef::of_key(key)?)
                }
                Level::Plan(_) => self.plan.index_of(key).map(|ix| self.placed.nodes[ix]),
                Level::Site => {
                    let site = self.site.as_ref()?;
                    let ix = site.index_of(key.strip_prefix("site:").unwrap_or(key))?;
                    Some(site.nodes[ix].rect)
                }
                Level::Domain => self
                    .domain_level()?
                    .regions
                    .iter()
                    .find(|r| &r.key == key)
                    .map(|r| r.rect),
            })
            .collect();
        let rect = rects.into_iter().reduce(|a, b| a.union(&b))?;
        let at = self.to_canvas((rect.right(), rect.y));
        Some(self.bounds.get().origin + at)
    }
}

/// The line (from 1) byte `offset` of a file's text is on.
fn line_of(text: Option<String>, offset: usize) -> usize {
    text.map(|text| 1 + text.bytes().take(offset).filter(|&b| b == b'\n').count())
        .unwrap_or(1)
}
