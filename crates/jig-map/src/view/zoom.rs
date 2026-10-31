//! The Map's one pan and zoom, and moving between levels. Pinching or
//! ⌘-scrolling zooms within a level; past its far end it steps out (Plan
//! to Site), and past its near end over a box it steps into it (a Site
//! box to its Plan, a Plan's function to its code, another function to
//! its Plan). Domain and Site are one drawing: zooming into a domain far
//! enough that it opens is Site, and zooming out of Site until no region
//! in view is open is Domain, the view staying where it is. A step
//! cross-fades: the old level is drawn fading out while
//! the new one fades in, the camera of each flying so that the box
//! entered grows into its contents, or what's left shrinks into the box
//! holding it. Once a gesture has changed level, it zooms no further
//! until it's let go.

use std::time::{Duration, Instant};

use gpui_kit::*;

use super::super::layout::Rect;
use super::super::plan::{Focus, Item};
use super::super::site::Kind;
use super::scene::Scene;
use super::{Level, MapEvent, MapView, has_plan};

/// Plan's zoom, within its level.
const PLAN_MIN: f32 = 0.3;
const PLAN_MAX: f32 = 2.;
/// Zoomed out past this, Plan steps out to Site: about half its own
/// size, or a little under where a big Plan starts out.
const PLAN_OUT: f32 = 0.5;
/// Zoomed in past this over a function, into it.
pub(super) const PLAN_IN: f32 = 1.9;
/// Site's closest zoom.
pub(super) const SITE_MAX: f32 = 3.;
/// A Site box filling this much of the canvas, zoomed into further,
/// steps into its Plan.
const SITE_IN: f32 = 0.7;
/// How long a level change takes, in seconds.
const FLIGHT: f32 = 0.45;
/// How long a pan within a level takes.
const PAN: f32 = 0.3;
/// How long boxes that left a Plan take to fade.
pub(super) const FADE_OUT: f32 = 0.25;
/// Zoomed into a region past this much of where it starts opening, Domain
/// is Site; zoomed out of Site past this much of where the region there
/// starts opening, it's Domain again. A little apart, so a pinch that
/// wavers doesn't flip between them.
pub(super) const DOMAIN_TO_SITE: f32 = 1.08;
pub(super) const SITE_TO_DOMAIN: f32 = 1.03;
/// How far ⌘+ and ⌘- zoom at a time.
const KEY_STEP: f32 = 1.25;
/// A gesture that changed level is over once it's quiet this long.
const GESTURE_GAP: Duration = Duration::from_millis(220);

/// What's in the middle of the canvas, in the drawing, and how far in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Camera {
    /// Where drawing rect `rect` is on a canvas `size` big.
    fn on_screen(&self, rect: Rect, (width, height): (f32, f32)) -> Rect {
        Rect {
            x: (rect.x - self.x) * self.zoom + width / 2.,
            y: (rect.y - self.y) * self.zoom + height / 2.,
            width: rect.width * self.zoom,
            height: rect.height * self.zoom,
        }
    }

    /// The camera that shows drawing rect `rect` filling screen rect
    /// `screen` (as near as its shape allows) on a canvas `size` big.
    pub(super) fn showing(rect: Rect, screen: Rect, (width, height): (f32, f32)) -> Camera {
        let zoom = (screen.width / rect.width.max(1.))
            .min(screen.height / rect.height.max(1.))
            .clamp(0.002, 400.);
        let (cx, cy) = rect.centre();
        let (sx, sy) = screen.centre();
        Camera {
            x: cx - (sx - width / 2.) / zoom,
            y: cy - (sy - height / 2.) / zoom,
            zoom,
        }
    }

    /// Part way from this camera to `to`: across in a line, zoom by
    /// ratio.
    fn towards(&self, to: Camera, t: f32) -> Camera {
        Camera {
            x: self.x + (to.x - self.x) * t,
            y: self.y + (to.y - self.y) * t,
            zoom: self.zoom * (to.zoom / self.zoom).powf(t),
        }
    }
}

/// A move from one view to another under way.
pub struct Transition {
    start: Instant,
    seconds: f32,
    /// The level left, fading out, and its camera from and to.
    old: Option<(Scene, Camera, Camera)>,
    /// The new view's camera from and to.
    from: Camera,
    to: Camera,
    /// Whether the new view fades in.
    fade_in: bool,
    /// Whether the gesture that started it zooms no further until it's let
    /// go: not between Domain and Site, which are one zoom.
    locks: bool,
}

impl Transition {
    /// Arriving from the code: flown out of the function, fading in.
    pub fn from_code(to: Camera) -> Transition {
        Transition {
            start: Instant::now(),
            seconds: FLIGHT,
            old: None,
            from: Camera {
                zoom: to.zoom * 3.,
                ..to
            },
            to,
            fade_in: true,
            locks: true,
        }
    }

    /// Within a level: the camera glides.
    pub fn pan(from: Camera, to: Camera) -> Transition {
        Transition {
            start: Instant::now(),
            seconds: PAN,
            old: None,
            from,
            to,
            fade_in: false,
            locks: true,
        }
    }

    /// From one level to another: the old one's `scene`, seen by
    /// `old_camera`, fades out as the new one fades in at `to`. With
    /// `matched`, a rect of the old drawing and one of the new that are
    /// the same place (a box and what's in it): the new level starts with
    /// its rect where the old one's is, and the old one ends with its
    /// rect where the new one's is.
    pub fn cross_fade(
        scene: Scene,
        old_camera: Camera,
        to: Camera,
        matched: Option<(Rect, Rect)>,
        size: (f32, f32),
    ) -> Transition {
        let (from, old_to) = match matched {
            Some((old, new)) => (
                Camera::showing(new, old_camera.on_screen(old, size), size),
                Camera::showing(old, to.on_screen(new, size), size),
            ),
            None => (
                Camera {
                    zoom: to.zoom * 0.6,
                    ..to
                },
                Camera {
                    zoom: old_camera.zoom * 1.6,
                    ..old_camera
                },
            ),
        };
        Transition {
            start: Instant::now(),
            seconds: FLIGHT,
            old: Some((scene, old_camera, old_to)),
            from,
            to,
            fade_in: true,
            locks: true,
        }
    }

    /// How far along it is at `now`, eased, from 0 to 1.
    fn progress(&self, now: Instant) -> f32 {
        let t = (now.duration_since(self.start).as_secs_f32() / self.seconds).min(1.);
        1. - (1. - t).powi(3)
    }

    fn done(&self, now: Instant) -> bool {
        now.duration_since(self.start).as_secs_f32() >= self.seconds
    }

    /// Whether another level is still fading in.
    pub fn fades(&self) -> bool {
        self.old.is_some() || self.fade_in
    }

    /// A move between Domain and Site, one zoom: the gesture carries on.
    pub fn carry_on(&mut self) {
        self.locks = false;
    }
}

/// How the canvas draws at a moment: the new view's camera and how
/// opaque it is, and the level left fading out, if one is.
pub struct Frame<'a> {
    pub camera: Camera,
    pub opacity: f32,
    pub old: Option<(&'a Scene, Camera, f32)>,
}

impl MapView {
    pub(super) fn canvas_size(&self) -> (f32, f32) {
        let size = self.bounds.get().size;
        if size.width > px(0.) {
            (f32::from(size.width), f32::from(size.height))
        } else {
            (1000., 600.)
        }
    }

    pub(super) fn camera(&self) -> Camera {
        let (width, height) = self.canvas_size();
        Camera {
            x: (width / 2. - f32::from(self.offset.x)) / self.zoom,
            y: (height / 2. - f32::from(self.offset.y)) / self.zoom,
            zoom: self.zoom,
        }
    }

    pub(super) fn set_camera(&mut self, camera: Camera) {
        let (width, height) = self.canvas_size();
        self.zoom = camera.zoom;
        self.offset = point(
            px(width / 2. - camera.x * camera.zoom),
            px(height / 2. - camera.y * camera.zoom),
        );
    }

    /// Moves the transition along: whether it's still under way.
    pub(super) fn step_transition(&mut self, now: Instant) -> bool {
        match &self.transition {
            Some(transition) if transition.done(now) => {
                self.transition = None;
                false
            }
            Some(_) => true,
            None => false,
        }
    }

    /// How the canvas draws at `now`.
    pub(super) fn frame(&self, now: Instant) -> Frame<'_> {
        let Some(transition) = &self.transition else {
            return Frame {
                camera: self.camera(),
                opacity: 1.,
                old: None,
            };
        };
        let t = transition.progress(now);
        Frame {
            camera: transition.from.towards(transition.to, t),
            opacity: if transition.fade_in { t } else { 1. },
            old: transition
                .old
                .as_ref()
                .map(|(scene, from, to)| (scene, from.towards(*to, t), 1. - t)),
        }
    }

    /// A point in the window, in the drawing.
    pub(super) fn to_drawing(&self, position: Point<Pixels>) -> (f32, f32) {
        let local = position - self.bounds.get().origin - self.offset;
        (
            f32::from(local.x) / self.zoom,
            f32::from(local.y) / self.zoom,
        )
    }

    /// A point in the drawing, in the canvas.
    pub(super) fn to_canvas(&self, (x, y): (f32, f32)) -> Point<Pixels> {
        point(
            self.offset.x + px(x * self.zoom),
            self.offset.y + px(y * self.zoom),
        )
    }

    /// Where the view is first put on a level: Site fits the project; a
    /// function's Plan centres on it; a module's fits what's there.
    pub(super) fn default_camera(&self) -> Camera {
        let (width, height) = self.canvas_size();
        match &self.level {
            Level::Site => self.site_fit(),
            Level::Domain => self.domain_fit(),
            Level::Plan(Focus::Function(_)) => self.centred_camera(1.),
            Level::Plan(Focus::Module(_)) => match self.placed.bounds() {
                Some(bounds) => {
                    let zoom = ((width - 80.) / bounds.width.max(1.))
                        .min((height - 100.) / bounds.height.max(1.))
                        .clamp(0.4, 1.);
                    let (x, y) = bounds.centre();
                    Camera { x, y, zoom }
                }
                None => Camera {
                    x: 0.,
                    y: 0.,
                    zoom: 1.,
                },
            },
        }
    }

    /// The Plan's function in the middle at `zoom`, else its middle.
    pub(super) fn centred_camera(&self, zoom: f32) -> Camera {
        let target = self
            .plan
            .focus
            .map(|focus| self.placed.nodes[focus])
            .or_else(|| self.placed.bounds());
        let (x, y) = target.map_or((0., 0.), |rect| rect.centre());
        Camera { x, y, zoom }
    }

    /// Site at its broadest: the region holding what's selected, else the
    /// biggest, filling the view and open.
    fn site_fit(&self) -> Camera {
        let Some(site) = self.site.as_ref() else {
            return Camera {
                x: 0.,
                y: 0.,
                zoom: 1.,
            };
        };
        let selected = self
            .selected
            .iter()
            .find_map(|key| site.index_of(key.strip_prefix("site:")?))
            .map(|ix| site.nodes[ix].region)
            .filter(|&ix| ix != 0);
        let biggest = site.regions().max_by(|&a, &b| {
            let area = |ix: usize| site.nodes[ix].rect.width * site.nodes[ix].rect.height;
            area(a).total_cmp(&area(b))
        });
        match selected.or(biggest) {
            Some(region) => self.camera_on_site_box(region),
            None => Camera {
                x: 0.,
                y: 0.,
                zoom: 1.,
            },
        }
    }

    /// How far out Site goes: past where every region in view has closed
    /// it's Domain. All of Site's regions, without a camera.
    pub(super) fn site_min(&self) -> f32 {
        let Some(site) = self.site.as_ref() else {
            return 0.01;
        };
        site.regions()
            .filter(|&ix| site.nodes[ix].opens())
            .map(|ix| site.nodes[ix].open_at)
            .fold(f32::INFINITY, f32::min)
            .min(SITE_MAX)
    }

    /// Whether any region in view at `zoom` around the camera now has
    /// started opening.
    fn any_region_open(&self, zoom: f32) -> bool {
        let Some(site) = self.site.as_ref() else {
            return false;
        };
        let (width, height) = self.canvas_size();
        let camera = self.camera();
        let view = Rect {
            x: camera.x - width / 2. / zoom,
            y: camera.y - height / 2. / zoom,
            width: width / zoom,
            height: height / zoom,
        };
        site.regions().any(|ix| {
            let node = &site.nodes[ix];
            node.rect.overlaps(&view) && node.opening(zoom) > 0.
        })
    }

    /// Site showing box `ix`: a box that opens filling the view, open; a
    /// file or type with the folder it's in, open, so its neighbours are
    /// in view too.
    pub(super) fn camera_on_site_box(&self, ix: usize) -> Camera {
        let (width, height) = self.canvas_size();
        let nodes = self.site_nodes();
        let node = &nodes[ix];
        let target = if node.opens() {
            ix
        } else {
            node.parent.unwrap_or(0)
        };
        // Open enough for the box and every box it's in.
        let mut need: f32 = 0.;
        let mut at = Some(target);
        while let Some(n) = at {
            if n != 0 {
                need = need.max(nodes[n].open_to() * 1.02);
            }
            at = nodes[n].parent;
        }
        let screen = Rect {
            x: 40.,
            y: 40.,
            width: width - 80.,
            height: height - 80.,
        };
        let fit = Camera::showing(nodes[target].rect, screen, (width, height));
        // No closer than Site's natural size, so what's around shows too.
        let zoom = fit.zoom.min(1.);
        if zoom >= need {
            return Camera { zoom, ..fit };
        }
        // Too big to fit open: centred on the box itself.
        let (x, y) = node.rect.centre();
        Camera {
            x,
            y,
            zoom: need.min(SITE_MAX),
        }
    }

    /// Zooming stays put while a gesture that changed level goes on.
    fn locked(&mut self, now: Instant) -> bool {
        let fading = self
            .transition
            .as_ref()
            .is_some_and(|transition| transition.fades() && transition.locks);
        let locked = fading
            || self
                .zoom_lock
                .is_some_and(|at| now.duration_since(at) < GESTURE_GAP);
        if locked {
            self.zoom_lock = Some(now);
        }
        locked
    }

    /// A pinch or ⌘-scroll by `factor` around `around`: zoomed within the
    /// level, or past its ends to the next.
    pub(super) fn zoom_at(&mut self, factor: f32, around: Point<Pixels>, cx: &mut Context<Self>) {
        let now = Instant::now();
        if self.locked(now) {
            return;
        }
        if self.transition.is_some() {
            self.transition = None;
        }
        let wanted = self.zoom * factor;
        match self.level.clone() {
            Level::Plan(_) => {
                if factor < 1. && wanted < self.plan_out() {
                    return self.step_out(cx);
                }
                if factor > 1.
                    && wanted > PLAN_IN
                    && let Some(ix) = self.plan_box_at(around)
                    && (self.plan.nodes[ix].function().is_some()
                        || matches!(self.plan.nodes[ix].item, Item::File { .. }))
                {
                    return self.enter_plan_box(ix, cx);
                }
                self.zoom_by(factor, around, PLAN_MIN, PLAN_MAX);
            }
            Level::Domain => {
                if factor > 1. && self.zoom_into_domain(wanted, around, cx) {
                    return;
                }
                let min = self.domain_min();
                self.zoom_by(factor, around, min, super::domain::DOMAIN_MAX);
            }
            Level::Site => {
                if factor < 1. && self.leaves_site(wanted, around) {
                    self.zoom_by(factor, around, 0., SITE_MAX);
                    self.continuous = true;
                    return self.show(Level::Domain, cx);
                }
                if factor > 1.
                    && let Some(ix) = self.site_box_at(around)
                {
                    let node = &self.site_nodes()[ix];
                    let (width, _) = self.canvas_size();
                    if has_plan(node.kind, node.functions)
                        && node.rect.width * wanted > SITE_IN * width
                    {
                        let place = node.place.clone();
                        return self.show(Level::Plan(Focus::Module(place)), cx);
                    }
                }
                self.zoom_by(factor, around, 0., SITE_MAX);
            }
        }
        self.moved(cx);
        cx.notify();
    }

    /// Where zooming out of this Plan goes to Site.
    pub(super) fn plan_out(&self) -> f32 {
        (self.default_camera().zoom * 0.85).clamp(PLAN_MIN + 0.02, PLAN_OUT)
    }

    /// Zoomed out at Site to `zoom` around `around`: Domain once the region
    /// there is closing, or, between regions, once none in view is open.
    fn leaves_site(&self, zoom: f32, around: Point<Pixels>) -> bool {
        let at = self.to_drawing(around);
        let region = self.site.as_ref().and_then(|site| {
            site.regions()
                .map(|ix| &site.nodes[ix])
                .find(|node| node.opens() && node.rect.contains(at))
        });
        match region {
            Some(node) => zoom < node.open_at * SITE_TO_DOMAIN,
            None => !self.any_region_open(zoom),
        }
    }

    /// ⌘+ and ⌘-: a step in or out around the middle, gliding there, and
    /// on past a level's end as a pinch would go.
    pub fn zoom_step(&mut self, closer: bool, cx: &mut Context<Self>) {
        let from = self.camera();
        let level = self.level.clone();
        let factor = if closer { KEY_STEP } else { 1. / KEY_STEP };
        self.zoom_at(factor, self.middle(), cx);
        if self.level == level && self.transition.is_none() {
            let to = self.camera();
            if to != from {
                self.transition = Some(Transition::pan(from, to));
                cx.notify();
            }
        }
    }

    /// ⌘0: the level as it first shows.
    pub fn zoom_reset(&mut self, cx: &mut Context<Self>) {
        let to = self.default_camera();
        self.transition = Some(Transition::pan(self.camera(), to));
        self.set_camera(to);
        self.save_view();
        cx.notify();
    }

    pub(super) fn zoom_by(&mut self, factor: f32, around: Point<Pixels>, min: f32, max: f32) {
        let zoom = (self.zoom * factor).clamp(min, max.max(min));
        let (x, y) = self.to_drawing(around);
        let local = around - self.bounds.get().origin;
        self.offset = point(local.x - px(x * zoom), local.y - px(y * zoom));
        self.zoom = zoom;
    }

    /// ⌘↑: Plan out to Site, Site out to Domain; Domain out to all of it.
    pub fn step_out(&mut self, cx: &mut Context<Self>) {
        match self.level {
            Level::Plan(_) => self.show(Level::Site, cx),
            Level::Site => self.show(Level::Domain, cx),
            Level::Domain => {
                let to = self.domain_fit();
                self.transition = Some(Transition::pan(self.camera(), to));
                self.set_camera(to);
                self.save_view();
                cx.notify();
            }
        }
    }

    /// The middle of the canvas, in the window.
    pub(super) fn middle(&self) -> Point<Pixels> {
        let (width, height) = self.canvas_size();
        self.bounds.get().origin + point(px(width / 2.), px(height / 2.))
    }

    /// ⌘↓ or Enter: into the box selected, else the one in the middle.
    pub fn step_in(&mut self, cx: &mut Context<Self>) {
        match self.level {
            Level::Plan(_) => {
                let picked = match self.selected_plan().as_slice() {
                    [ix] => Some(*ix),
                    _ => self.plan.focus,
                };
                if let Some(ix) = picked {
                    self.enter_plan_box(ix, cx);
                }
            }
            Level::Domain => self.step_into_domain(cx),
            Level::Site => {
                let picked = self
                    .selected
                    .iter()
                    .find_map(|key| self.site.as_ref()?.index_of(key.strip_prefix("site:")?))
                    .or_else(|| {
                        let (width, height) = self.canvas_size();
                        let middle =
                            self.bounds.get().origin + point(px(width / 2.), px(height / 2.));
                        self.site_box_at(middle)
                    });
                if let Some(ix) = picked {
                    self.enter_site_box(ix, cx);
                }
            }
        }
    }

    /// Into Site box `ix`: a folder flown into until it's open and fills
    /// the view; a file or a type to the Plan of its functions; a file
    /// the index doesn't read opened as it is.
    pub fn enter_site_box(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(node) = self.site_nodes().get(ix).cloned() else {
            return;
        };
        match node.kind {
            _ if has_plan(node.kind, node.functions) => {
                self.show(Level::Plan(Focus::Module(node.place)), cx)
            }
            Kind::File { .. } | Kind::Type => {
                let path = node.place.split('#').next().unwrap_or(&node.place);
                cx.emit(MapEvent::OpenFile(self.root.join(path)));
            }
            _ => {
                let to = self.camera_on_site_box(ix);
                self.transition = Some(Transition::pan(self.camera(), to));
                self.set_camera(to);
                self.selected = vec![super::site_key(&node.key)];
                self.save_view();
                cx.notify();
            }
        }
    }

    /// Scrolled or zoomed: kept once it settles.
    pub(super) fn moved(&mut self, cx: &mut Context<Self>) {
        let at = Instant::now();
        self.scrolled = Some(at);
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(600))
                .await;
            this.update(cx, |this, _| {
                if this.scrolled == Some(at) {
                    this.scrolled = None;
                    this.save_view();
                }
            })
            .ok();
        })
        .detach();
    }

    /// For tests: the gesture that changed level has ended.
    #[cfg(any(test, feature = "testing"))]
    pub fn end_gesture(&mut self) {
        self.zoom_lock = None;
        self.transition = None;
    }
}
