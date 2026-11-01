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
mod moving;
pub mod ruler;
mod scene;
pub mod sketch;
mod sketch_draw;
#[cfg(any(test, feature = "testing"))]
mod testing;
mod zoom;
use moving::*;
mod arrangement;
mod live;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use gpui_kit::*;
use serde_json::{Value, json};

use crate::domain::debug::{self, Firing, Stack};
use crate::domain::domains::actions::Action;
use crate::domain::index::{FnId, Project, Target};
use crate::domain::layout::{self, Placed, Rect};
use crate::domain::plan::{self, Focus, Item, Opened, Plan, function_key, slashed};
use crate::domain::site::{Kind, Show, Site};
use crate::store::cache::Folder;
use crate::store::persist::places;
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
    /// Where the ruler's needle was last drawn, easing to where it goes.
    ruler: Cell<Option<f32>>,
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
            ruler: Cell::new(None),
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
        let plan = plan::plan(&project, &Folder(&project.root), focus, &self.opened);
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

    fn site_nodes(&self) -> &[crate::domain::site::Node] {
        self.site.as_ref().map_or(&[], |site| &site.nodes)
    }

    fn gliding(&self, now: Instant) -> bool {
        self.glide
            .as_ref()
            .is_some_and(|(_, start)| now.duration_since(*start).as_secs_f32() < GLIDE)
    }

    fn stack_functions(&self) -> Vec<FnId> {
        match &self.project {
            Some(project) => debug::on_stack(project, &self.stack),
            None => Vec::new(),
        }
    }

    /// Functions glowing as the program enters them, or not.
    pub fn set_live(&mut self, on: bool, cx: &mut Context<Self>) {
        self.live = on;
        self.firing.clear();
        cx.emit(MapEvent::LiveLines);
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
type SiteArrows = Rc<Vec<crate::domain::site::Arrow>>;

/// A Site box's key as selections keep it, apart from Plan's.
fn site_key(key: &str) -> String {
    format!("site:{key}")
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
