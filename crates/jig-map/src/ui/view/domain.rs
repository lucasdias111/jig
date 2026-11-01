//! The Domain level on the canvas. Before Index Domains has been kept it
//! shows Jig's own grouping as drafts under a banner; after, the saved
//! domains, each a region in its own colour, its name, purpose and offers
//! only. Domain and Site are one drawing: zooming into a region carries on
//! into Site, the region opening onto its share of the code where it is;
//! zooming out of Site closes them again.
//!
//! Index Domains, Update Domains and splitting a domain start a normal
//! conversation with the agent; its answer shows here as a proposal in
//! the agent's colour, to keep, edit or discard, and is saved only once
//! kept. Domains are map data, not code, so they're edited right here:
//! rename and purpose in place, drag a file onto another region, drop one
//! domain onto another to merge them. Each edit saves at once, and ⌘Z
//! undoes it. Once saved, each domain's colour tints its boxes at Site and
//! Plan, which the header's switch turns off.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use super::scene::{Arrow, RegionBox, Scene};
use super::zoom::{Camera, Transition};
use super::{Level, MapEvent, MapView};
use crate::domain::domains::actions::{self, Action};
use crate::domain::domains::grouping::Grouping;
use crate::domain::domains::level::{DomainLevel, RegionKind};
use crate::domain::domains::{Domains, Home};
use crate::domain::layout::Rect;
use crate::domain::plan::{Focus, Item};
use crate::domain::site::{Kind, Site};
use crate::store::persist;
mod agent_actions;
mod editing;
mod regions;
#[cfg(any(test, feature = "testing"))]
mod testing;

/// Domain's closest zoom.
pub(super) const DOMAIN_MAX: f32 = 3.;

#[derive(Default)]
pub(super) struct DomainState {
    /// As `.jig/map/domains.toml` has them.
    pub saved: Option<Domains>,
    /// Jig's own grouping, worked out when it's first needed.
    draft: RefCell<Option<Rc<Grouping>>>,
    pub proposal: Option<Proposal>,
    /// The level as laid out, until what it shows changes.
    level: RefCell<Option<Rc<DomainLevel>>>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Tints at Site and Plan turned off.
    pub tints_off: bool,
    /// Each file's and type's saved domain, for the tints.
    homes: HashMap<String, usize>,
    /// An action's conversation under way.
    pub working: Option<Action>,
    pub editing: Option<Editing>,
}

/// What an action's answer proposes, until it's kept or discarded.
#[derive(Clone)]
pub(super) struct Proposal {
    pub action: Action,
    pub domains: Domains,
}

/// The domains before an edit, to go back to.
#[derive(Clone)]
struct Snapshot {
    saved: Option<Domains>,
    proposal: Option<Proposal>,
}

/// A domain's name or purpose being edited in place.
pub(super) struct Editing {
    /// The domain, by its place in the domains.
    pub domain: usize,
    pub field: Field,
    pub input: Entity<InputState>,
    _events: Subscription,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    Purpose,
}

/// Domain number `ix`'s colour: distinct hues, none the agent's orange.
pub fn colour(ix: usize, dark: bool) -> Hsla {
    const HUES: [f32; 10] = [212., 146., 276., 186., 52., 340., 104., 242., 312., 166.];
    let turn = (ix / HUES.len()) as f32 * 17.;
    let hue = (HUES[ix % HUES.len()] + turn) % 360.;
    Hsla {
        h: hue / 360.,
        s: 0.55,
        l: if dark { 0.62 } else { 0.46 },
        a: 1.,
    }
}

impl MapView {
    /// The saved domains read again, and what's drawn from them.
    pub(super) fn reload_domains(&mut self) {
        self.domains.saved = Domains::read(&self.root);
        *self.domains.draft.borrow_mut() = None;
        self.domains_changed();
    }

    /// What the domains show changed: Site and Domain laid out again,
    /// the tints too.
    pub(super) fn domains_changed(&mut self) {
        *self.domains.level.borrow_mut() = None;
        *self.site_arrows.borrow_mut() = None;
        self.label_spots.borrow_mut().clear();
        self.domains.homes.clear();
        if let (Some(project), Some(domains)) = (self.project.clone(), self.shown_domains()) {
            let mut site = Site::of(&project, &domains, self.site_show);
            // Purposes kept from the agent's descriptions win over the
            // code's own.
            let kept = crate::store::persist::purposes::read(&self.root);
            for node in &mut site.nodes {
                if let Some(purpose) = kept.purposes.get(&node.place) {
                    node.purpose = Some(purpose.clone());
                }
            }
            self.site = Some(Rc::new(site));
        }
        let (Some(saved), Some(site)) = (&self.domains.saved, &self.site) else {
            return;
        };
        for node in &site.nodes {
            if matches!(node.kind, Kind::File { .. } | Kind::Type)
                && let Home::Domain(ix) = saved.home_of(&node.place)
            {
                self.domains.homes.insert(node.place.clone(), ix);
            }
        }
    }

    /// Jig's grouping of the project, worked out once.
    fn grouping(&self) -> Option<Rc<Grouping>> {
        let project = self.project.as_ref()?;
        let mut draft = self.domains.draft.borrow_mut();
        Some(
            draft
                .get_or_insert_with(|| Rc::new(Grouping::of(project)))
                .clone(),
        )
    }

    /// The domains showing: a proposal, else the saved ones, else Jig's
    /// drafts.
    pub(super) fn shown_domains(&self) -> Option<Domains> {
        if let Some(proposal) = &self.domains.proposal {
            return Some(proposal.domains.clone());
        }
        if let Some(saved) = &self.domains.saved {
            return Some(saved.clone());
        }
        self.grouping().map(|grouping| grouping.domains())
    }

    /// Whether the Domain level shows Jig's drafts.
    pub(super) fn drafts(&self) -> bool {
        self.domains.proposal.is_none() && self.domains.saved.is_none()
    }

    /// The Domain level as laid out.
    pub(super) fn domain_level(&self) -> Option<Rc<DomainLevel>> {
        if let Some(level) = self.domains.level.borrow().as_ref() {
            return Some(level.clone());
        }
        let project = self.project.clone()?;
        let domains = self.shown_domains()?;
        let site = self.site.clone()?;
        let level = Rc::new(DomainLevel::of(&project, &domains, self.drafts(), &site));
        *self.domains.level.borrow_mut() = Some(level.clone());
        Some(level)
    }

    /// The saved domain a Site or Plan box at `place` is tinted with.
    pub(super) fn tint_of(&self, place: &str) -> Option<usize> {
        if self.domains.tints_off {
            return None;
        }
        self.domains.homes.get(place).copied()
    }

    /// Whether there are saved domains to tint with.
    pub(super) fn has_tints(&self) -> bool {
        self.domains.saved.is_some()
    }

    pub fn set_tints(&mut self, on: bool, cx: &mut Context<Self>) {
        self.domains.tints_off = !on;
        cx.notify();
    }

    pub(super) fn domain_min(&self) -> f32 {
        self.domain_fit().zoom * 0.8
    }

    /// The key a Domain box under `position` is selected by: its region's.
    pub(super) fn domain_key_at(&self, position: Point<Pixels>) -> Option<String> {
        let level = self.domain_level()?;
        self.region_at(position)
            .map(|ix| level.regions[ix].key.clone())
    }

    /// The Site boxes `boxes` cover, all of them.
    pub(super) fn site_boxes_rect(&self, boxes: &[usize]) -> Option<Rect> {
        Rect::around(boxes.iter().map(|&ix| self.site_nodes()[ix].rect))
    }

    /// An action's conversation is working on its answer.
    pub fn working_on_action(&mut self, action: Action, cx: &mut Context<Self>) {
        self.domains.working = Some(action);
        cx.notify();
    }

    /// The action's conversation ended without an answer.
    pub fn action_ended(&mut self, cx: &mut Context<Self>) {
        if self.domains.working.take().is_some() {
            cx.notify();
        }
    }

    /// File or type `place` moved by hand into region `to`.
    pub(super) fn move_to_region(&mut self, place: &str, to: Home, cx: &mut Context<Self>) {
        let place = place.to_string();
        self.edit_domains(|domains| domains.move_member(&place, to), cx);
    }

    /// Domain `from` dropped onto domain `into`: merged.
    pub(super) fn merge_domains(&mut self, from: usize, into: usize, cx: &mut Context<Self>) {
        self.edit_domains(|domains| domains.merge(from, into), cx);
        self.selected.clear();
    }

    /// The domain region `ix` of the level showing stands for.
    pub(super) fn domain_of_region(&self, ix: usize) -> Option<usize> {
        match self.domain_level()?.regions.get(ix)?.kind {
            RegionKind::Domain(domain) => Some(domain),
            _ => None,
        }
    }

    /// A short notice in the canvas's hint line.
    pub(super) fn say(&mut self, text: &str, cx: &mut Context<Self>) {
        self.notice = Some((text.to_string(), std::time::Instant::now()));
        cx.notify();
    }

    /// The tint for a Plan function's box: its file's or type's domain.
    pub(super) fn plan_tint(&self, item: &Item) -> Option<usize> {
        let Item::Function(id) = item else {
            return None;
        };
        let project = self.project.as_ref()?;
        let function = &project.functions[*id];
        let file = crate::domain::domains::place_of(&project.files[function.file].path);
        function
            .outline
            .owner
            .as_ref()
            .and_then(|owner| self.tint_of(&format!("{file}#{owner}")))
            .or_else(|| self.tint_of(&file))
    }

    /// A Site box's key, for the domains' moves.
    pub(super) fn site_place(&self, key: &str) -> Option<String> {
        let ix = self.site.as_ref()?.index_of(key.strip_prefix("site:")?)?;
        let node = &self.site_nodes()[ix];
        matches!(node.kind, Kind::File { .. } | Kind::Type).then(|| node.place.clone())
    }
}

/// The link between two regions, its flows labelled.
pub(super) fn link_arrow(level: &DomainLevel, link: &crate::domain::domains::level::Link) -> Arrow {
    let (a, b) = (level.regions[link.a].rect, level.regions[link.b].rect);
    let (from, to) = super::scene::edge_points(a, b);
    let labels = link
        .forward
        .iter()
        .map(|label| (true, label.clone()))
        .chain(link.backward.iter().map(|label| (false, label.clone())))
        .collect();
    Arrow {
        from,
        to,
        keys: (
            level.regions[link.a].key.clone(),
            level.regions[link.b].key.clone(),
        ),
        across: false,
        dashed: level.draft,
        calls: None,
        alpha: 1.,
        heads: (!link.backward.is_empty(), !link.forward.is_empty()),
        labels,
        link: true,
        faint: false,
        picked: false,
        tint: None,
        title: None,
        title_tint: None,
        lit: None,
    }
}

/// The link between two regions as drawn with region `picked` (by key)
/// selected: one of its links is drawn in its colour, headed by the
/// domain at the other end and read from its side; any other link's
/// words are faint.
pub(super) fn picked_link(
    level: &DomainLevel,
    link: &crate::domain::domains::level::Link,
    picked: Option<&str>,
) -> Arrow {
    let mut arrow = link_arrow(level, link);
    let (a, b) = (&level.regions[link.a], &level.regions[link.b]);
    let (mine, other) = if Some(a.key.as_str()) == picked {
        (a, b)
    } else if Some(b.key.as_str()) == picked {
        (b, a)
    } else {
        arrow.faint = true;
        return arrow;
    };
    let colour = |region: &crate::domain::domains::level::Region| match region.kind {
        RegionKind::Domain(ix) if !level.draft => Some(ix),
        _ => None,
    };
    arrow.picked = true;
    arrow.tint = colour(mine);
    arrow.title = Some(other.name.clone());
    arrow.title_tint = colour(other);
    // Its words run from `a` to `b`; read from the selected side.
    if std::ptr::eq(mine, b) {
        for (forward, _) in &mut arrow.labels {
            *forward = !*forward;
        }
    }
    arrow
}

fn plural(count: usize, what: &str) -> String {
    format!("{count} {what}{}", if count == 1 { "" } else { "s" })
}
