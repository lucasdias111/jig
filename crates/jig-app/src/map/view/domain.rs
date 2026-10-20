//! The Domain level on the canvas. Before Index Domains has been kept it
//! shows Jig's own grouping as drafts under a banner; after, the saved
//! domains, each a region in its own colour, sized by its code, its files
//! fading in as you zoom into it. Zooming into a region, or one of its
//! files, goes to Site there; out of Site comes back here.
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

use super::super::domains::actions::{self, Action};
use super::super::domains::grouping::Grouping;
use super::super::domains::level::{DomainLevel, RegionKind};
use super::super::domains::{Domains, Home};
use super::super::layout::Rect;
use super::super::plan::{Focus, Item};
use super::super::site::Kind;
use super::scene::{Arrow, Card, Look, RegionBox, Scene};
use super::zoom::{Camera, Transition};
use super::{Level, MapEvent, MapView};

/// Domain's closest zoom.
pub(super) const DOMAIN_MAX: f32 = 3.;
/// A region this wide on screen starts showing its files, and shows them
/// fully at the second.
pub const OPEN_FROM: f32 = 640.;
pub const OPEN_TO: f32 = 860.;
/// Zoomed in over a file so it's this much of the canvas wide: into Site.
const MEMBER_IN: f32 = 0.45;
/// Zoomed in over a region past this many canvas widths: into Site.
const REGION_IN: f32 = 2.6;

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

/// How far a region `width` wide on screen shows its files, from 0 to 1.
pub fn opening(width: f32) -> f32 {
    let t = ((width - OPEN_FROM) / (OPEN_TO - OPEN_FROM)).clamp(0., 1.);
    t * t * (3. - 2. * t)
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

    /// What the domains show changed: laid out again, the tints too.
    fn domains_changed(&mut self) {
        *self.domains.level.borrow_mut() = None;
        self.domains.homes.clear();
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
        let level = Rc::new(DomainLevel::of(&project, &domains, self.drafts()));
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

    /// The whole Domain level in view.
    pub(super) fn domain_fit(&self) -> Camera {
        let (width, height) = self.canvas_size();
        let Some(bounds) = self.domain_level().and_then(|level| level.bounds()) else {
            return Camera {
                x: 0.,
                y: 0.,
                zoom: 1.,
            };
        };
        let top = if self.domain_banner().is_some() {
            56.
        } else {
            24.
        };
        let zoom = ((width - 60.) / bounds.width.max(1.))
            .min((height - 40. - top) / bounds.height.max(1.))
            .min(1.);
        let (x, y) = bounds.centre();
        Camera {
            x,
            y: y - (top - 24.) / 2. / zoom,
            zoom,
        }
    }

    pub(super) fn domain_min(&self) -> f32 {
        self.domain_fit().zoom * 0.8
    }

    /// Domain as `camera` sees it: each region, its files fading in as
    /// it grows on screen, and one link between each pair that has flows.
    pub(super) fn domain_scene(&self, camera: Camera) -> Scene {
        let mut scene = Scene::default();
        let Some(level) = self.domain_level() else {
            return scene;
        };
        let (width, height) = self.canvas_size();
        let view = Rect {
            x: camera.x - width / 2. / camera.zoom,
            y: camera.y - height / 2. / camera.zoom,
            width: width / camera.zoom,
            height: height / camera.zoom,
        };
        let proposed = self.domains.proposal.is_some();
        for (ix, region) in level.showing() {
            let open = opening(region.rect.width * camera.zoom);
            scene.regions.push(RegionBox {
                key: region.key.clone(),
                rect: region.rect,
                name: region.name.clone(),
                purpose: region.purpose.clone(),
                offers: region.offers.clone(),
                why: region.why.clone(),
                note: region.detail(),
                kind: region.kind,
                draft: level.draft,
                proposed: proposed && matches!(region.kind, RegionKind::Domain(_)),
                open,
                alpha: 1.,
                ix,
            });
            if open <= 0. || !region.rect.overlaps(&view) {
                continue;
            }
            let tint = match region.kind {
                RegionKind::Domain(ix) if !level.draft => Some(ix),
                _ => None,
            };
            for member in &region.members {
                let look = if member.place.contains('#') {
                    Look::Type
                } else if member.read {
                    Look::File
                } else {
                    Look::Unread
                };
                scene.cards.push(Card {
                    key: member.key.clone(),
                    rect: member.rect,
                    title: member.title.clone(),
                    detail: member.folder.clone(),
                    note: if member.read {
                        plural(member.functions, "function")
                    } else {
                        "not read".into()
                    },
                    badges: Vec::new(),
                    look,
                    alpha: open,
                    focus: false,
                    hidden: [0; 3],
                    unknown_target: false,
                    checked: false,
                    tint,
                    ix,
                });
            }
        }
        for link in &level.links {
            let (a, b) = (level.regions[link.a].rect, level.regions[link.b].rect);
            let (from, to) = super::scene::edge_points(a, b);
            let labels = link
                .forward
                .iter()
                .map(|label| (true, label.clone()))
                .chain(link.backward.iter().map(|label| (false, label.clone())))
                .collect();
            scene.arrows.push(Arrow {
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
            });
        }
        scene
    }

    /// The region under `position`, by its place in the level.
    pub(super) fn region_at(&self, position: Point<Pixels>) -> Option<usize> {
        let at = self.to_drawing(position);
        let level = self.domain_level()?;
        level
            .showing()
            .find(|(_, region)| region.rect.contains(at))
            .map(|(ix, _)| ix)
    }

    /// The file or type under `position`, if its region shows them: its
    /// region and place.
    pub(super) fn member_at(&self, position: Point<Pixels>) -> Option<(usize, String)> {
        let at = self.to_drawing(position);
        let drawn = self.drawn.borrow();
        let card = drawn
            .cards
            .iter()
            .rev()
            .find(|card| card.key.starts_with("member:") && card.rect.contains(at))?;
        Some((card.ix, card.key["member:".len()..].to_string()))
    }

    /// The key a Domain box under `position` is selected by: a file's or
    /// type's, else its region's.
    pub(super) fn domain_key_at(&self, position: Point<Pixels>) -> Option<String> {
        if let Some((_, place)) = self.member_at(position) {
            return Some(format!("member:{place}"));
        }
        let level = self.domain_level()?;
        self.region_at(position)
            .map(|ix| level.regions[ix].key.clone())
    }

    /// Zoomed in at Domain far enough over a file or a region: into Site
    /// there. Whether it went.
    pub(super) fn zoom_into_domain(
        &mut self,
        wanted: f32,
        around: Point<Pixels>,
        cx: &mut Context<Self>,
    ) -> bool {
        let (width, _) = self.canvas_size();
        if let Some((_, place)) = self.member_at(around) {
            let rect = self
                .domain_level()
                .and_then(|level| member_rect(&level, &place));
            if rect.is_some_and(|rect| rect.width * wanted > MEMBER_IN * width) {
                self.enter_member(&place, cx);
                return true;
            }
            return false;
        }
        let Some(ix) = self.region_at(around) else {
            return false;
        };
        let level = self.domain_level().expect("a region");
        if level.regions[ix].rect.width * wanted > REGION_IN * width {
            self.enter_region(ix, cx);
            return true;
        }
        false
    }

    /// ⌘↓ or Enter at Domain: into the file or region selected, else the
    /// region in the middle.
    pub(super) fn step_into_domain(&mut self, cx: &mut Context<Self>) {
        let Some(level) = self.domain_level() else {
            return;
        };
        if let Some(place) = self
            .selected
            .iter()
            .find_map(|key| key.strip_prefix("member:"))
        {
            let place = place.to_string();
            return self.enter_member(&place, cx);
        }
        let picked = self
            .selected
            .iter()
            .find_map(|key| level.regions.iter().position(|region| &region.key == key))
            .or_else(|| self.region_at(self.middle()));
        if let Some(ix) = picked {
            self.enter_region(ix, cx);
        }
    }

    /// Into Site from region `ix`: where its files are.
    pub(super) fn enter_region(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(level) = self.domain_level() else {
            return;
        };
        let region = &level.regions[ix];
        let boxes: Vec<usize> = region
            .members
            .iter()
            .filter_map(|member| self.site.as_ref()?.box_of(&member.place))
            .collect();
        self.arriving_at = Some((region.rect, boxes));
        self.show(Level::Site, cx);
    }

    /// Into Site from file or type `place`: its box.
    pub(super) fn enter_member(&mut self, place: &str, cx: &mut Context<Self>) {
        let Some(level) = self.domain_level() else {
            return;
        };
        let Some(rect) = member_rect(&level, place) else {
            return;
        };
        let boxes: Vec<usize> = self
            .site
            .as_ref()
            .and_then(|site| site.box_of(place))
            .into_iter()
            .collect();
        self.arriving_at = Some((rect, boxes));
        self.show(Level::Site, cx);
    }

    /// The Site boxes `boxes` cover, all of them.
    pub(super) fn site_boxes_rect(&self, boxes: &[usize]) -> Option<Rect> {
        Rect::around(boxes.iter().map(|&ix| self.site_nodes()[ix].rect))
    }

    /// Site showing all of `boxes`: one box as entering it does, several
    /// fitting the view.
    pub(super) fn camera_on_site_boxes(&self, boxes: &[usize]) -> Camera {
        match boxes {
            [] => self.default_camera(),
            [one] => self.camera_on_site_box(*one),
            _ => {
                let (width, height) = self.canvas_size();
                let rect = self.site_boxes_rect(boxes).expect("boxes");
                let screen = Rect {
                    x: 40.,
                    y: 40.,
                    width: width - 80.,
                    height: height - 80.,
                };
                let to = Camera::showing(rect, screen, (width, height));
                Camera {
                    zoom: to.zoom.clamp(self.site_min(), 3.),
                    ..to
                }
            }
        }
    }

    /// Coming out to Domain from `left`: the region holding what was
    /// selected or focused there, selected (you are here).
    pub(super) fn domain_of_selection(&self, left: &Level) -> Vec<String> {
        let Some(level) = self.domain_level() else {
            return Vec::new();
        };
        let place = match left {
            Level::Plan(Focus::Function(symbol)) => Some(symbol.file.clone()),
            Level::Plan(Focus::Module(place)) => Some(place.clone()),
            Level::Site => self.selected.iter().find_map(|key| {
                let key = key.strip_prefix("site:")?;
                let ix = self.site.as_ref()?.index_of(key)?;
                Some(self.site_nodes()[ix].place.clone())
            }),
            Level::Domain => None,
        };
        let Some(place) = place else {
            return Vec::new();
        };
        let region = level.region_of(&place).or_else(|| {
            // A folder: the region holding the most of it.
            let mut counts: HashMap<usize, usize> = HashMap::new();
            for (ix, region) in level.regions.iter().enumerate() {
                for member in &region.members {
                    if member.place.starts_with(&place) {
                        *counts.entry(ix).or_default() += 1;
                    }
                }
            }
            counts
                .into_iter()
                .max_by_key(|&(ix, n)| (n, std::cmp::Reverse(ix)))
                .map(|(ix, _)| ix)
        });
        region
            .map(|ix| vec![level.regions[ix].key.clone()])
            .unwrap_or_default()
    }

    /// The banner over Domain, if it has one: drafts to index, an action
    /// under way, or a proposal to review.
    pub(super) fn domain_banner(&self) -> Option<String> {
        if self.level != Level::Domain {
            return None;
        }
        if let Some(action) = &self.domains.working {
            return Some(format!(
                "{} is under way in the agent's conversation…",
                action.title()
            ));
        }
        if let Some(proposal) = &self.domains.proposal {
            return Some(match &proposal.action {
                Action::Index => "The agent proposes these domains".into(),
                Action::Update(files) => format!(
                    "The agent proposes where {} go{}",
                    plural(files.len(), "new file"),
                    if files.len() == 1 { "es" } else { "" }
                ),
                Action::Split(name) => format!("The agent proposes how to split {name}"),
            });
        }
        if self.drafts() {
            return Some("Index Domains to name and describe these".into());
        }
        None
    }

    /// Start a Map action's conversation: what it asks, from the domains
    /// as they are.
    pub fn start_action(&mut self, action: Action, cx: &mut Context<Self>) {
        let instruction = match &action {
            Action::Index => {
                let Some(grouping) = self.grouping() else {
                    return;
                };
                actions::index_instruction(&grouping.hint())
            }
            Action::Update(files) => {
                let Some(saved) = &self.domains.saved else {
                    return;
                };
                if files.is_empty() {
                    return self.say("Every file has a domain: there's nothing to place.", cx);
                }
                actions::update_instruction(files, saved)
            }
            Action::Split(name) => {
                let Some(saved) = &self.domains.saved else {
                    return;
                };
                actions::split_instruction(name, saved)
            }
        };
        self.domains.working = Some(action.clone());
        cx.emit(MapEvent::Ask(action, instruction));
        cx.notify();
    }

    /// Update Domains: the agent places what's unsorted.
    pub fn update_domains(&mut self, cx: &mut Context<Self>) {
        let files = self
            .domain_level()
            .map(|level| level.unsorted())
            .unwrap_or_default();
        self.start_action(Action::Update(files), cx);
    }

    /// The agent answered action `action` with `reply`: its proposal shows,
    /// or why it can't.
    pub fn answer(
        &mut self,
        action: Action,
        reply: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), String> {
        self.domains.working = None;
        let proposal = action
            .proposal(reply, self.domains.saved.as_ref())
            .map_err(|error| format!("{error:#}"));
        if let Ok(domains) = &proposal {
            self.remember();
            self.domains.proposal = Some(Proposal {
                action,
                domains: domains.clone(),
            });
            self.domains_changed();
            if self.level == Level::Domain {
                let to = self.domain_fit();
                self.transition = Some(Transition::pan(self.camera(), to));
                self.set_camera(to);
            }
        }
        cx.notify();
        proposal.map(|_| ())
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

    /// Keep the proposal: saved, and showing as the domains.
    pub fn keep_proposal(&mut self, cx: &mut Context<Self>) {
        let Some(proposal) = self.domains.proposal.clone() else {
            return;
        };
        self.remember();
        self.domains.proposal = None;
        self.save_domains(Some(proposal.domains), cx);
    }

    /// Discard the proposal; ⌘Z brings it back.
    pub fn discard_proposal(&mut self, cx: &mut Context<Self>) {
        if self.domains.proposal.is_none() {
            return;
        }
        self.remember();
        self.domains.proposal = None;
        self.domains_changed();
        cx.notify();
    }

    /// The domains now, kept for ⌘Z.
    fn remember(&mut self) {
        self.domains.undo.push(Snapshot {
            saved: self.domains.saved.clone(),
            proposal: self.domains.proposal.clone(),
        });
        self.domains.redo.clear();
    }

    /// `saved` written to `domains.toml` (or the file removed, for none)
    /// and shown.
    fn save_domains(&mut self, saved: Option<Domains>, cx: &mut Context<Self>) {
        let path = self.root.join(super::super::domains::FILE);
        let written = match &saved {
            Some(domains) => domains.write(&self.root).map_err(|e| format!("{e:#}")),
            None => std::fs::remove_file(&path).or(Ok(())),
        };
        if let Err(error) = written {
            self.say(&format!("Couldn't save the domains: {error}"), cx);
        }
        self.domains.saved = saved;
        super::keep_out_of_git(&self.root);
        self.domains_changed();
        cx.notify();
    }

    /// An edit by hand: to the proposal while there is one, else to the
    /// saved domains, which are written at once. ⌘Z undoes it.
    fn edit_domains(&mut self, edit: impl FnOnce(&mut Domains), cx: &mut Context<Self>) {
        if let Some(proposal) = &self.domains.proposal {
            let mut domains = proposal.domains.clone();
            edit(&mut domains);
            if domains == proposal.domains {
                return;
            }
            self.remember();
            if let Some(proposal) = &mut self.domains.proposal {
                proposal.domains = domains;
            }
            self.domains_changed();
            cx.notify();
            return;
        }
        let Some(saved) = self.domains.saved.clone() else {
            return self.say(
                "These are Jig's drafts: Index Domains first, then edit what it finds.",
                cx,
            );
        };
        let mut domains = saved.clone();
        edit(&mut domains);
        if domains == saved {
            return;
        }
        self.remember();
        self.save_domains(Some(domains), cx);
    }

    /// ⌘Z: the domains as they were before the last edit.
    pub fn undo_domains(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(snapshot) = self.domains.undo.pop() else {
            return false;
        };
        self.domains.redo.push(Snapshot {
            saved: self.domains.saved.clone(),
            proposal: self.domains.proposal.clone(),
        });
        self.restore(snapshot, cx);
        true
    }

    /// ⇧⌘Z: the edit undone, again.
    pub fn redo_domains(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(snapshot) = self.domains.redo.pop() else {
            return false;
        };
        self.domains.undo.push(Snapshot {
            saved: self.domains.saved.clone(),
            proposal: self.domains.proposal.clone(),
        });
        self.restore(snapshot, cx);
        true
    }

    fn restore(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.domains.proposal = snapshot.proposal;
        if snapshot.saved != self.domains.saved {
            self.save_domains(snapshot.saved, cx);
        } else {
            self.domains_changed();
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

    /// Splitting a domain asks the agent.
    pub fn split_domain(&mut self, domain: usize, cx: &mut Context<Self>) {
        let Some(name) = self
            .domains
            .saved
            .as_ref()
            .and_then(|saved| saved.domains.get(domain))
            .map(|domain| domain.name.clone())
        else {
            return;
        };
        self.start_action(Action::Split(name), cx);
    }

    /// The domain region `ix` of the level showing stands for.
    pub(super) fn domain_of_region(&self, ix: usize) -> Option<usize> {
        match self.domain_level()?.regions.get(ix)?.kind {
            RegionKind::Domain(domain) => Some(domain),
            _ => None,
        }
    }

    /// Edit domain `domain`'s name or purpose in place.
    pub fn begin_edit(
        &mut self,
        domain: usize,
        field: Field,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.drafts() {
            return self.say(
                "These are Jig's drafts: Index Domains first, then edit what it finds.",
                cx,
            );
        }
        let Some(shown) = self.shown_domains() else {
            return;
        };
        let Some(was) = shown.domains.get(domain) else {
            return;
        };
        let value = match field {
            Field::Name => was.name.clone(),
            Field::Purpose => was.purpose.clone(),
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
        let events = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::PressEnter { .. } => {
                this.finish_edit(true, cx);
                this.focus.focus(window, cx);
            }
            InputEvent::Blur => this.finish_edit(true, cx),
            _ => {}
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.domains.editing = Some(Editing {
            domain,
            field,
            input,
            _events: events,
        });
        cx.notify();
    }

    /// The edit in place done: kept, or given up.
    pub fn finish_edit(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(editing) = self.domains.editing.take() else {
            return;
        };
        let text = editing.input.read(cx).value().to_string();
        if keep {
            let (domain, field) = (editing.domain, editing.field);
            self.edit_domains(
                |domains| {
                    if domain >= domains.domains.len() {
                        return;
                    }
                    match field {
                        Field::Name => domains.rename(domain, &text),
                        Field::Purpose => domains.set_purpose(domain, &text),
                    }
                },
                cx,
            );
            // The region's key is its name: keep it selected.
            if field == Field::Name
                && let Some(level) = self.domain_level()
                && let Some(region) = level
                    .regions
                    .iter()
                    .find(|region| region.kind == RegionKind::Domain(domain))
            {
                self.selected = vec![region.key.clone()];
            }
        }
        cx.notify();
    }

    /// Whether a field of the Map's has the keyboard.
    pub fn editing(&self, window: &Window, cx: &App) -> bool {
        self.domains
            .editing
            .as_ref()
            .is_some_and(|editing| editing.input.focus_handle(cx).is_focused(window))
    }

    /// Copy as Markdown: each domain's name, purpose and offers, then the
    /// flows.
    pub fn copy_markdown(&mut self, cx: &mut Context<Self>) {
        let Some(domains) = self.shown_domains() else {
            return;
        };
        let project = self
            .root
            .file_name()
            .map_or("Project".into(), |name| name.to_string_lossy().into_owned());
        cx.write_to_clipboard(ClipboardItem::new_string(domains.markdown(&project)));
        self.say("Copied the domains as Markdown.", cx);
    }

    /// A short notice in the canvas's hint line.
    pub(super) fn say(&mut self, text: &str, cx: &mut Context<Self>) {
        self.notice = Some((text.to_string(), std::time::Instant::now()));
        cx.notify();
    }

    /// Where a dragged file or type would go if dropped at `position`: the
    /// home there, at Domain its region, at Site the domain of the box.
    pub(super) fn drop_home(&self, position: Point<Pixels>) -> Option<(Home, String)> {
        match self.level {
            Level::Domain => {
                let level = self.domain_level()?;
                let ix = self.region_at(position)?;
                let region = &level.regions[ix];
                (region.kind != RegionKind::Unsorted).then(|| (region.home(), region.name.clone()))
            }
            Level::Site => {
                let saved = self.domains.saved.as_ref()?;
                let ix = self.site_box_at(position)?;
                let node = &self.site_nodes()[ix];
                match saved.home_of(&node.place) {
                    Home::Domain(domain) => {
                        Some((Home::Domain(domain), saved.domains[domain].name.clone()))
                    }
                    Home::Shared => Some((Home::Shared, "Shared".into())),
                    Home::Unsorted => None,
                }
            }
            Level::Plan(_) => None,
        }
    }

    /// The home `place` has in the domains showing.
    pub(super) fn home_now(&self, place: &str) -> Home {
        match &self.domains.proposal {
            Some(proposal) => proposal.domains.home_of(place),
            None => self
                .domains
                .saved
                .as_ref()
                .map_or(Home::Unsorted, |saved| saved.home_of(place)),
        }
    }

    /// The tint for a Plan function's box: its file's or type's domain.
    pub(super) fn plan_tint(&self, item: &Item) -> Option<usize> {
        let Item::Function(id) = item else {
            return None;
        };
        let project = self.project.as_ref()?;
        let function = &project.functions[*id];
        let file = super::super::domains::place_of(&project.files[function.file].path);
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

/// File or type `place`'s box at Domain.
fn member_rect(level: &DomainLevel, place: &str) -> Option<Rect> {
    let region = level.region_of(place)?;
    level.regions[region]
        .members
        .iter()
        .find(|member| member.place == place)
        .map(|member| member.rect)
}

fn plural(count: usize, what: &str) -> String {
    format!("{count} {what}{}", if count == 1 { "" } else { "s" })
}

#[cfg(test)]
impl MapView {
    /// Site with the whole project in view.
    pub fn fit_site(&mut self) {
        let to = self.default_camera();
        self.set_camera(to);
    }

    /// Site close in on box `key`, the folder it's in open.
    pub fn show_site_box(&mut self, key: &str) {
        let ix = self
            .site
            .as_ref()
            .and_then(|s| s.index_of(key))
            .expect("a box");
        let to = self.camera_on_site_box(ix);
        self.set_camera(to);
    }

    pub fn domain_regions(&self) -> Vec<String> {
        self.domain_level()
            .map(|level| level.showing().map(|(_, r)| r.name.clone()).collect())
            .unwrap_or_default()
    }

    pub fn has_proposal(&self) -> bool {
        self.domains.proposal.is_some()
    }

    pub fn banner(&self) -> Option<String> {
        self.domain_banner()
    }

    /// Where region `name` is in the window: its middle, or a point just
    /// inside its top left, past the name.
    pub fn region_point(&self, name: &str, middle: bool) -> Point<Pixels> {
        let level = self.domain_level().expect("domains");
        let region = level
            .regions
            .iter()
            .find(|region| region.name == name)
            .unwrap_or_else(|| panic!("no region {name}"));
        let at = if middle {
            region.rect.centre()
        } else {
            (
                region.rect.x + 30. / self.zoom,
                region.rect.y + 12. / self.zoom,
            )
        };
        self.bounds.get().origin + self.to_canvas(at)
    }

    pub fn domain_input(&self) -> Option<Entity<InputState>> {
        self.domains.editing.as_ref().map(|e| e.input.clone())
    }

    pub fn tint_at(&self, place: &str) -> Option<usize> {
        self.tint_of(place)
    }
}
