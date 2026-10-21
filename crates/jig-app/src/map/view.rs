//! The Map's canvas, in place of the code: one pan and zoom over three
//! levels, Plan, Site and Domain. Pinch or ⌘-scroll zooms within a level; past a
//! threshold it cross-fades to the next, the box you enter growing into
//! its contents or the one you leave shrinking into its parent; ⌘↑ and
//! ⌘↓ step a level the same way, and Enter or a double click enters the
//! selected box. Zooming into a Plan's own function lands in its code.
//!
//! Entering the Map, each level stepped into and each re-centring is a
//! place for Go Back. Every view keeps how it was arranged
//! (`.jig/map/places.json`, which Jig keeps out of git): where Plan's
//! boxes were dragged, the hops opened, where it was scrolled and zoomed
//! to. Tidy lays a Plan out afresh. The canvas never changes code.

mod ask;
mod domain;
mod domain_draw;
mod draw;
mod export;
mod input;
pub mod lens;
mod lens_draw;
mod scene;
pub mod sketch;
mod sketch_draw;
mod zoom;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::*;
use serde_json::{Value, json};

use super::debug::{self, Firing, Stack};
use super::domains::actions::Action;
use super::index::{FnId, Project, Target};
use super::layout::{self, Placed, Rect};
use super::plan::{self, Focus, Item, Opened, Plan, function_key, slashed};
use super::site::{Kind, Show, Site};
pub use lens_draw::ProposalChoice;
use scene::Scene;
use zoom::{Camera, Transition};

actions!(map, [KeepDescriptions]);

/// The Map's own keys that GPUI would otherwise take: Tab keeps the
/// agent's descriptions rather than moving the focus.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new("tab", KeepDescriptions, Some("Map"))]
}

/// How long boxes take to glide to their new places.
const GLIDE: f32 = 0.22;

/// Where boxes were drawn, by key, and when they started gliding from
/// there.
type Glide = (HashMap<String, (f32, f32)>, Instant);

/// Which level of the Map shows.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Level {
    /// A neighbourhood: of a function, or of a module.
    Plan(Focus),
    /// The whole project, nested.
    Site,
    /// The whole project as business areas.
    Domain,
}

impl Level {
    /// What the level is called, for the header and its place in Go Back.
    pub fn name(&self) -> &'static str {
        match self {
            Level::Plan(_) => "Plan",
            Level::Site => "Site",
            Level::Domain => "Domain",
        }
    }
}

pub enum MapEvent {
    /// The Map moved on from this level, for Go Back.
    Moved(Level),
    /// Into a function's code: its file, and where its name is.
    OpenCode(PathBuf, usize),
    /// A file the index doesn't read, opened as it is.
    OpenFile(PathBuf),
    /// Ask the language server what this function calls and is called by.
    AskServer(FnId),
    /// The functions on the Plan changed: what to hear about as the
    /// program runs.
    LiveLines,
    /// Escape with nothing to do: back to the code.
    Close,
    /// Start a conversation for a Map action: what it is, and what the
    /// agent is asked.
    Ask(Action, String),
    /// Explain this level: the agent is asked, with the level as context.
    Explain(String),
    /// The agent's descriptions in the conversation's drawing (by its id)
    /// kept, or not.
    Settled(usize, bool),
    /// What to do with the conversation's proposal (by its id).
    Proposal(usize, lens_draw::ProposalChoice),
    /// Start a conversation named `.0`, the agent asked `.1` about what's
    /// selected: a sketch's Make Task.
    Converse(String, String),
    /// Implement the planner task `.0`, made from sketch `.1`.
    Implement(String, String),
    /// Show the planner task with this id.
    OpenTask(String),
}

pub struct MapView {
    focus: FocusHandle,
    root: PathBuf,
    project: Option<Arc<Project>>,
    site: Option<Rc<Site>>,
    level: Level,
    /// What a Plan opened past its first hop.
    opened: Opened,
    plan: Plan,
    placed: Placed,
    /// Where boxes were put by hand on the Plan showing, by key.
    places: HashMap<String, (f32, f32)>,
    offset: Point<Pixels>,
    zoom: f32,
    bounds: Rc<Cell<Bounds<Pixels>>>,
    selected: Vec<String>,
    drag: Option<input::Drag>,
    /// The Plan's boxes as last drawn, gliding to their new places.
    glide: Option<Glide>,
    /// Boxes that left the Plan on a re-centring, fading where they were.
    leaving: Option<(Scene, Instant)>,
    /// A level changing: the one left, fading out.
    transition: Option<Transition>,
    /// Zooming stays put until the gesture that changed level ends.
    zoom_lock: Option<Instant>,
    /// When it was last scrolled or zoomed, not yet kept.
    scrolled: Option<Instant>,
    /// What the pointer is over, and since when.
    hover: Option<input::Hover>,
    /// The scene as last drawn, for finding what's under the pointer.
    drawn: RefCell<Scene>,
    /// Site's arrows for the boxes showing, kept while they stay the same.
    site_arrows: RefCell<Option<(u64, SiteArrows)>>,
    /// Where each link's words were put, in the drawing, and how big they
    /// were: kept so they stay put as the view moves.
    label_spots: RefCell<HashMap<String, LabelSpot>>,
    /// Tests shown at Site.
    site_show: Show,
    /// The level is changing by zooming on, not by a jump: the view stays
    /// where it is, since Domain and Site share their drawing.
    continuous: bool,
    /// The paused program's stack, and its functions on the Plan.
    stack: Stack,
    on_stack: Vec<FnId>,
    /// Functions glowing as the program enters them.
    live: bool,
    firing: Firing,
    notice: Option<(String, Instant)>,
    /// The domains: saved, Jig's drafts, a proposal, and editing them.
    domains: domain::DomainState,
    /// Going into Site from Domain: the box entered, and the Site boxes it
    /// stands for.
    arriving_at: Option<(Rect, Vec<usize>)>,
    /// What the agent drew, and the pins.
    lens: lens::LensState,
    /// The user's sketches, and the tool picked.
    sketches: sketch::SketchState,
}

impl EventEmitter<MapEvent> for MapView {}

impl Focusable for MapView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl MapView {
    /// The Map of the project at `root`, at Site until it's told where.
    pub fn new(root: PathBuf, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle(),
            root,
            project: None,
            site: None,
            level: Level::Site,
            opened: Opened::default(),
            plan: Plan::default(),
            placed: Placed::default(),
            places: HashMap::new(),
            offset: point(px(60.), px(60.)),
            zoom: 1.,
            bounds: Default::default(),
            selected: Vec::new(),
            drag: None,
            glide: None,
            leaving: None,
            transition: None,
            zoom_lock: None,
            scrolled: None,
            hover: None,
            drawn: RefCell::new(Scene::default()),
            site_arrows: RefCell::new(None),
            label_spots: RefCell::new(HashMap::new()),
            site_show: Show::default(),
            continuous: false,
            stack: Vec::new(),
            on_stack: Vec::new(),
            live: true,
            firing: Firing::default(),
            notice: None,
            domains: domain::DomainState::default(),
            arriving_at: None,
            lens: lens::LensState::default(),
            sketches: sketch::SketchState::default(),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn project(&self) -> Option<&Arc<Project>> {
        self.project.as_ref()
    }

    pub fn level(&self) -> &Level {
        &self.level
    }

    /// The project as read again: drawn anew, staying where it was.
    pub fn set_project(&mut self, project: Arc<Project>, cx: &mut Context<Self>) {
        // The function of a Plan may have moved or been renamed.
        if let Level::Plan(Focus::Function(symbol)) = &self.level
            && let Some(id) = project.find(symbol)
        {
            let found = project.symbol(id);
            if &found != symbol {
                self.level = Level::Plan(Focus::Function(found));
            }
        }
        self.project = Some(project);
        self.lens.forget_places();
        self.reload_pins();
        self.reload_domains();
        self.rebuild(true);
        self.reload_sketches();
        self.on_stack = self.stack_functions();
        self.ask_server(cx);
        cx.emit(MapEvent::LiveLines);
        cx.notify();
    }

    /// Tests, or calls into Shared, shown at Site or not: Site laid out
    /// again.
    pub fn set_site_show(&mut self, show: Show, cx: &mut Context<Self>) {
        if show == self.site_show {
            return;
        }
        self.site_show = show;
        self.domains_changed();
        cx.notify();
    }

    /// Show `level`, the level left remembered for Go Back.
    pub fn show(&mut self, level: Level, cx: &mut Context<Self>) {
        if level != self.level {
            cx.emit(MapEvent::Moved(self.level.clone()));
        }
        self.go_to(level, cx);
    }

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

    /// Ask the language server about the Plan's function, once.
    fn ask_server(&self, cx: &mut Context<Self>) {
        if let Level::Plan(Focus::Function(symbol)) = &self.level
            && let Some(project) = &self.project
            && let Some(id) = project.find(symbol)
            && !project.checked(id)
        {
            cx.emit(MapEvent::AskServer(id));
        }
    }

    /// What the language server says function `id` calls and is called
    /// by: drawn with certainty.
    pub fn server_calls(
        &mut self,
        id: FnId,
        callees: Vec<Target>,
        callers: Vec<FnId>,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self.project.as_mut() else {
            return;
        };
        Arc::make_mut(project).merge_server(id, callees, callers);
        self.rebuild(true);
        cx.notify();
    }

    /// The Plan for the level showing, laid out where boxes were put.
    fn rebuild(&mut self, glide: bool) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let Level::Plan(focus) = &self.level else {
            self.plan = Plan::default();
            self.placed = Placed::default();
            return;
        };
        let was = self.positions();
        let plan = plan::plan(&project, focus, &self.opened);
        self.placed = layout::layout(&plan);
        self.plan = plan;
        for (ix, node) in self.plan.nodes.iter().enumerate() {
            if let Some(&place) = self.places.get(&node.key) {
                self.placed.put(ix, place);
            }
        }
        self.placed.refit(&self.plan);
        self.glide = (glide && !was.is_empty()).then(|| (was, Instant::now()));
        let keys: Vec<&String> = self.plan.nodes.iter().map(|n| &n.key).collect();
        self.selected.retain(|key| {
            keys.contains(&key) || key.starts_with("site:") || key.starts_with("sketch:")
        });
    }

    /// Where each of the Plan's boxes is now, by key.
    fn positions(&self) -> HashMap<String, (f32, f32)> {
        self.plan
            .nodes
            .iter()
            .zip(&self.placed.nodes)
            .map(|(node, rect)| (node.key.clone(), (rect.x, rect.y)))
            .collect()
    }

    /// Another Plan: the box re-centred on stays put, the boxes still
    /// there glide to their new places, new ones slide in and those out
    /// of range fade where they were.
    fn recentre(&mut self, level: Level, cx: &mut Context<Self>) {
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
    fn step_to(&mut self, level: Level, cx: &mut Context<Self>) {
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
    fn after_moving(&mut self, cx: &mut Context<Self>) {
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
    fn entered_rect(&self, level: &Level) -> Option<Rect> {
        let Level::Plan(focus) = level else {
            return None;
        };
        match &self.level {
            Level::Site => self.site_box_of(focus).map(|ix| self.site_nodes()[ix].rect),
            Level::Plan(_) | Level::Domain => None,
        }
    }

    /// The Site box holding what a Plan is about.
    fn site_box_of(&self, focus: &Focus) -> Option<usize> {
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

    fn site_nodes(&self) -> &[super::site::Node] {
        self.site.as_ref().map_or(&[], |site| &site.nodes)
    }

    /// The old level's whole drawing, or the box in it that matters.
    fn content_rect(&self) -> Option<Rect> {
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

    /// Where views keep how they were arranged.
    fn places_file(&self) -> PathBuf {
        self.root.join(".jig/map/places.json")
    }

    /// The view showing, as its arrangement is kept.
    fn view_key(&self) -> String {
        match &self.level {
            Level::Site => "site".into(),
            Level::Domain => "domain".into(),
            Level::Plan(Focus::Module(place)) => format!("plan:{place}"),
            Level::Plan(Focus::Function(symbol)) => {
                format!("plan:{}#{}", symbol.file, symbol.title())
            }
        }
    }

    /// The view's arrangement as it was kept: its places and hops opened,
    /// and where it was scrolled and zoomed to, if it was.
    fn load_view(&mut self) -> Option<Camera> {
        self.places.clear();
        self.opened = Opened::default();
        let saved = read_json(&self.places_file()).map(|all| all[self.view_key()].clone())?;
        if let Some(Value::Object(places)) = saved.get("places") {
            for (key, place) in places {
                if let (Some(x), Some(y)) = (place[0].as_f64(), place[1].as_f64()) {
                    self.places.insert(key.clone(), (x as f32, y as f32));
                }
            }
        }
        if let Some(opened) = saved.get("opened") {
            self.opened = serde_json::from_value(opened.clone()).unwrap_or_default();
        }
        let viewport = saved.get("viewport")?;
        Some(Camera {
            x: viewport[0].as_f64()? as f32,
            y: viewport[1].as_f64()? as f32,
            zoom: viewport[2].as_f64()? as f32,
        })
    }

    /// Keep how the view showing is arranged.
    pub fn save_view(&self) {
        if self.project.is_none() {
            return;
        }
        let places: serde_json::Map<String, Value> = self
            .places
            .iter()
            .map(|(key, (x, y))| (key.clone(), json!([x, y])))
            .collect();
        let file = self.places_file();
        let mut all = match read_json(&file) {
            Some(Value::Object(all)) => all,
            _ => serde_json::Map::new(),
        };
        let camera = self.camera();
        all.insert(
            self.view_key(),
            json!({
                "places": places,
                "opened": self.opened,
                "viewport": [camera.x, camera.y, camera.zoom],
            }),
        );
        write_json(&file, &Value::Object(all));
        keep_out_of_git(&self.root);
    }

    /// Tidy: the Plan laid out afresh.
    pub fn tidy(&mut self, cx: &mut Context<Self>) {
        self.places.clear();
        self.rebuild(true);
        let to = self.default_camera();
        self.transition = Some(Transition::pan(self.camera(), to));
        self.set_camera(to);
        self.save_view();
        cx.notify();
    }

    /// The layout as drawn now, gliding after a change.
    fn drawn_plan(&self, now: Instant) -> Placed {
        let mut placed = self.placed.clone();
        let Some((was, start)) = &self.glide else {
            return placed;
        };
        let t = (now.duration_since(*start).as_secs_f32() / GLIDE).min(1.);
        if t >= 1. {
            return placed;
        }
        let eased = 1. - (1. - t).powi(3);
        for (ix, node) in self.plan.nodes.iter().enumerate() {
            if let Some(&(x, y)) = was.get(&node.key) {
                let rect = &mut placed.nodes[ix];
                rect.x = x + (rect.x - x) * eased;
                rect.y = y + (rect.y - y) * eased;
            }
        }
        placed.refit(&self.plan);
        placed
    }

    /// How far boxes new to the Plan have faded in, from 0 to 1.
    fn arriving(&self, key: &str, now: Instant) -> f32 {
        match &self.glide {
            Some((was, start)) if !was.contains_key(key) => {
                (now.duration_since(*start).as_secs_f32() / GLIDE).min(1.)
            }
            _ => 1.,
        }
    }

    fn gliding(&self, now: Instant) -> bool {
        self.glide
            .as_ref()
            .is_some_and(|(_, start)| now.duration_since(*start).as_secs_f32() < GLIDE)
    }

    /// The debugger paused with this call stack (empty: running, or not
    /// debugging): its functions light up as a path. Pausing in a
    /// function not on the Plan shows its Plan.
    pub fn set_stack(&mut self, stack: Stack, cx: &mut Context<Self>) {
        if stack == self.stack {
            return;
        }
        self.stack = stack;
        self.on_stack = self.stack_functions();
        if let (Some(&paused), Some(project)) = (self.on_stack.first(), self.project.clone())
            && matches!(self.level, Level::Plan(_))
            && self
                .plan
                .index_of(&function_key(&project, paused))
                .is_none()
        {
            self.show(Level::Plan(Focus::Function(project.symbol(paused))), cx);
        }
        cx.notify();
    }

    fn stack_functions(&self) -> Vec<FnId> {
        match &self.project {
            Some(project) => debug::on_stack(project, &self.stack),
            None => Vec::new(),
        }
    }

    /// The lines (0-based) the program should report reaching as it
    /// runs, by file: the first statement of each function on the Plan.
    pub fn live_lines(&self) -> Vec<(PathBuf, u32)> {
        let (Some(project), true, Level::Plan(_)) = (&self.project, self.live, &self.level) else {
            return Vec::new();
        };
        self.plan
            .nodes
            .iter()
            .filter_map(Node::function)
            .filter_map(|id| {
                let function = &project.functions[id];
                let line = function.outline.body_line?;
                Some((self.root.join(&project.files[function.file].path), line))
            })
            .collect()
    }

    /// Functions glowing as the program enters them, or not.
    pub fn set_live(&mut self, on: bool, cx: &mut Context<Self>) {
        self.live = on;
        self.firing.clear();
        cx.emit(MapEvent::LiveLines);
        cx.notify();
    }

    /// The program reached line `line` (0-based) of `path`: the function
    /// starting there glows.
    pub fn fire_line(&mut self, path: &Path, line: u32, cx: &mut Context<Self>) {
        let Some(project) = self.project.clone() else {
            return;
        };
        let Some(id) = debug::entered(&project, &debug::relative(&self.root, path), line) else {
            return;
        };
        let key = function_key(&project, id);
        if self.plan.index_of(&key).is_none() {
            return;
        }
        let plan = &self.plan;
        self.firing.fire(&key, Instant::now(), |from, to| {
            match (plan.index_of(from), plan.index_of(to)) {
                (Some(from), Some(to)) => plan.wires.iter().any(|w| w.from == from && w.to == to),
                _ => false,
            }
        });
        cx.notify();
    }
}

/// Which hop a box's expand button opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expand {
    Callers,
    Callees,
    Implementations,
}

use plan::Node;

/// Where a link's words were put, in the drawing, and their size on
/// screen.
type LabelSpot = ((f32, f32), (f32, f32));

/// Site's arrows for the boxes showing.
type SiteArrows = Rc<Vec<super::site::Arrow>>;

/// A Site box's key as selections keep it, apart from Plan's.
fn site_key(key: &str) -> String {
    format!("site:{key}")
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn write_json(path: &Path, value: &Value) {
    if let Some(folder) = path.parent() {
        let _ = std::fs::create_dir_all(folder);
    }
    let text = serde_json::to_string_pretty(value).unwrap_or_default();
    let _ = std::fs::write(path, text + "\n");
}

/// `.jig/.gitignore` keeps the Map's places out of git: they're how one
/// person arranged things, not the project's.
fn keep_out_of_git(root: &Path) {
    let file = root.join(".jig/.gitignore");
    let line = "map/places.json";
    let text = std::fs::read_to_string(&file).unwrap_or_default();
    if text.lines().any(|l| l.trim() == line) {
        return;
    }
    let mut text = text;
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(line);
    text.push('\n');
    let _ = std::fs::write(file, text);
}

impl Render for MapView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let flying = self.step_transition(now);
        if flying
            || self.gliding(now)
            || self.firing.lit(now)
            || self
                .leaving
                .as_ref()
                .is_some_and(|(_, at)| now.duration_since(*at).as_secs_f32() < zoom::FADE_OUT)
        {
            window.request_animation_frame();
        }
        gpui_kit::component::v_flex()
            .key_context("Map")
            .track_focus(&self.focus)
            .size_full()
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(|this, _: &KeepDescriptions, _, cx| {
                match this.descriptions_waiting() {
                    Some(source) => this.settle_descriptions(source, true, cx),
                    None => cx.propagate(),
                }
            }))
            .child(self.render_header(cx))
            .child(self.render_canvas(now, window, cx))
    }
}

#[cfg(test)]
impl MapView {
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    pub fn site(&self) -> Option<&Site> {
        self.site.as_deref()
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn selected_keys(&self) -> &[String] {
        &self.selected
    }

    /// Whether a level is still cross-fading in.
    pub fn crossfading(&self) -> bool {
        self.transition.as_ref().is_some_and(Transition::fades)
    }

    /// The Plan box titled `title`.
    pub fn titled(&self, title: &str) -> usize {
        self.plan
            .nodes
            .iter()
            .position(|node| node.title == title)
            .unwrap_or_else(|| {
                panic!(
                    "no {title} in {:?}",
                    self.plan.nodes.iter().map(|n| &n.title).collect::<Vec<_>>()
                )
            })
    }

    /// Where Plan box `ix`'s header is in the window, for tests to press.
    pub fn node_point(&self, ix: usize) -> Point<Pixels> {
        let rect = self.placed.nodes[ix];
        self.bounds.get().origin
            + self.to_canvas((
                rect.x + rect.width / 2.,
                rect.y + layout::HEADER_HEIGHT / 2.,
            ))
    }

    /// Where Site box `key` is in the window, its middle.
    pub fn site_point(&self, key: &str) -> Point<Pixels> {
        let site = self.site.as_ref().expect("read");
        let rect = site.nodes[site.index_of(key).expect("a box")].rect;
        self.bounds.get().origin + self.to_canvas(rect.centre())
    }

    /// The Site boxes showing, by key, and whether each is open.
    pub fn site_showing(&self) -> Vec<(String, bool)> {
        self.drawn
            .borrow()
            .frames
            .iter()
            .map(|frame| (frame.key.clone(), true))
            .chain(
                self.drawn
                    .borrow()
                    .cards
                    .iter()
                    .map(|card| (card.key.clone(), false)),
            )
            .collect()
    }

    /// The words on the arrows drawn: flows, and calls leaving a region.
    pub fn drawn_labels(&self) -> Vec<String> {
        self.drawn
            .borrow()
            .arrows
            .iter()
            .flat_map(|arrow| arrow.labels.iter().map(|(_, label)| label.clone()))
            .collect()
    }

    /// Whether each drawn link's words are faint.
    pub fn drawn_faint(&self) -> Vec<bool> {
        self.drawn
            .borrow()
            .arrows
            .iter()
            .filter(|a| a.link)
            .map(|a| a.faint)
            .collect()
    }

    /// How many links between domains are drawn.
    pub fn drawn_links(&self) -> usize {
        self.drawn.borrow().arrows.iter().filter(|a| a.link).count()
    }

    /// The arrows drawn at Site between boxes, by their ends' keys, and
    /// whether each is lit for the box selected (`None`: nothing is).
    pub fn site_arrows_lit(&self) -> Vec<((String, String), Option<bool>)> {
        self.drawn
            .borrow()
            .arrows
            .iter()
            .filter(|arrow| !arrow.link)
            .map(|arrow| (arrow.keys.clone(), arrow.lit))
            .collect()
    }

    /// The regions drawn lit, by key.
    pub fn lit_regions(&self) -> Vec<String> {
        self.drawn
            .borrow()
            .regions
            .iter()
            .filter(|region| region.lit)
            .map(|region| region.key.clone())
            .collect()
    }

    /// The headings of the selected domain's links drawn: the domains at
    /// their other ends.
    pub fn picked_links(&self) -> Vec<String> {
        self.drawn
            .borrow()
            .arrows
            .iter()
            .filter(|arrow| arrow.picked)
            .filter_map(|arrow| arrow.title.clone())
            .collect()
    }

    /// The regions drawn, by key, and how far each is open.
    pub fn drawn_regions(&self) -> Vec<(String, Rect, f32)> {
        self.drawn
            .borrow()
            .regions
            .iter()
            .map(|region| (region.key.clone(), region.rect, region.open))
            .collect()
    }

    /// Where box `key` of Site is drawn in the window, if it's in view.
    pub fn site_box_on_screen(&self, key: &str) -> Option<Rect> {
        let site = self.site.as_ref()?;
        let rect = site.nodes[site.index_of(key)?].rect;
        let at = self.to_canvas((rect.x, rect.y));
        Some(Rect {
            x: f32::from(at.x),
            y: f32::from(at.y),
            width: rect.width * self.zoom,
            height: rect.height * self.zoom,
        })
    }

    /// The Plan's functions lit as the debugger's path.
    pub fn lit_path(&self) -> Vec<String> {
        let project = self.project.as_ref().expect("read");
        self.on_stack
            .iter()
            .map(|&id| project.functions[id].title())
            .collect()
    }

    pub fn firing_of(&self, title: &str) -> f32 {
        let key = &self.plan.nodes[self.titled(title)].key;
        self.firing.of_box(key, Instant::now())
    }
}

/// Whether `kind` is a box that opens into a Plan: of its functions, or
/// of a file the index can't read as a whole.
fn has_plan(kind: Kind, functions: usize) -> bool {
    match kind {
        Kind::File { read: false } => true,
        Kind::File { read: true } | Kind::Type => functions > 0,
        _ => false,
    }
}
