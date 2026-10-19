//! The Map's one pan and zoom, and moving between levels. Pinching or
//! ⌘-scrolling zooms within a level; past its far end it steps out (Plan
//! to Site), and past its near end over a box it steps into it (a Site
//! box to its Plan, a Plan's function to its code, another function to
//! its Plan). A step cross-fades: the old level is drawn fading out while
//! the new one fades in, the camera of each flying so that the box
//! entered grows into its contents, or what's left shrinks into the box
//! holding it. Once a gesture has changed level, it zooms no further
//! until it's let go.

use std::time::{Duration, Instant};

use gpui_kit::*;

use super::super::layout::Rect;
use super::super::plan::Focus;
use super::super::site::Kind;
use super::scene::Scene;
use super::{Level, MapEvent, MapView, has_plan};

/// Plan's zoom, within its level.
const PLAN_MIN: f32 = 0.3;
const PLAN_MAX: f32 = 2.;
/// Zoomed out past this, Plan steps out to Site.
const PLAN_OUT: f32 = 0.32;
/// Zoomed in past this over a function, into it.
const PLAN_IN: f32 = 1.9;
/// Site's closest zoom.
const SITE_MAX: f32 = 3.;
/// A Site box filling this much of the canvas, zoomed into further,
/// steps into its Plan.
const SITE_IN: f32 = 0.7;
/// How long a level change takes, in seconds.
const FLIGHT: f32 = 0.45;
/// How long a pan within a level takes.
const PAN: f32 = 0.3;
/// How long boxes that left a Plan take to fade.
pub(super) const FADE_OUT: f32 = 0.25;
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
    fn showing(rect: Rect, screen: Rect, (width, height): (f32, f32)) -> Camera {
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

    /// The whole project in view.
    fn site_fit(&self) -> Camera {
        let (width, height) = self.canvas_size();
        let Some(root) = self.site_nodes().first() else {
            return Camera {
                x: 0.,
                y: 0.,
                zoom: 1.,
            };
        };
        let zoom = ((width - 60.) / root.rect.width.max(1.))
            .min((height - 60.) / root.rect.height.max(1.))
            .min(1.);
        let (x, y) = root.rect.centre();
        Camera { x, y, zoom }
    }

    /// How far out Site goes: the whole project, with room round it.
    fn site_min(&self) -> f32 {
        self.site_fit().zoom * 0.8
    }

    /// Site centred on box `ix`, close enough that the box it's in is
    /// open and it's a good size.
    pub(super) fn camera_on_site_box(&self, ix: usize) -> Camera {
        let (width, _) = self.canvas_size();
        let nodes = self.site_nodes();
        let rect = nodes[ix].rect;
        let mut zoom = 0.3 * width / rect.width.max(1.);
        if let Some(parent) = nodes[ix].parent.filter(|&p| p != 0) {
            zoom = zoom.max(super::scene::OPEN_TO * 1.05 / nodes[parent].rect.width.max(1.));
        }
        let zoom = zoom.clamp(self.site_min(), SITE_MAX);
        let (x, y) = rect.centre();
        Camera { x, y, zoom }
    }

    /// Zooming stays put while a gesture that changed level goes on.
    fn locked(&mut self, now: Instant) -> bool {
        let fading = self.transition.as_ref().is_some_and(Transition::fades);
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
                if factor < 1. && wanted < PLAN_OUT {
                    return self.step_out(cx);
                }
                if factor > 1.
                    && wanted > PLAN_IN
                    && let Some(ix) = self.plan_box_at(around)
                    && self.plan.nodes[ix].function().is_some()
                {
                    return self.enter_plan_box(ix, cx);
                }
                self.zoom_by(factor, around, PLAN_MIN, PLAN_MAX);
            }
            Level::Site => {
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
                let min = self.site_min();
                self.zoom_by(factor, around, min, SITE_MAX);
            }
        }
        self.moved(cx);
        cx.notify();
    }

    pub(super) fn zoom_by(&mut self, factor: f32, around: Point<Pixels>, min: f32, max: f32) {
        let zoom = (self.zoom * factor).clamp(min, max.max(min));
        let (x, y) = self.to_drawing(around);
        let local = around - self.bounds.get().origin;
        self.offset = point(local.x - px(x * zoom), local.y - px(y * zoom));
        self.zoom = zoom;
    }

    /// ⌘↑: Plan out to Site; Site out to the whole project.
    pub fn step_out(&mut self, cx: &mut Context<Self>) {
        match self.level {
            Level::Plan(_) => self.show(Level::Site, cx),
            Level::Site => {
                let to = self.site_fit();
                self.transition = Some(Transition::pan(self.camera(), to));
                self.set_camera(to);
                self.save_view();
                cx.notify();
            }
        }
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
            Kind::Root | Kind::Folder { .. } => {
                let (width, height) = self.canvas_size();
                let screen = Rect {
                    x: 40.,
                    y: 40.,
                    width: width - 80.,
                    height: height - 80.,
                };
                let to = Camera::showing(node.rect, screen, (width, height));
                let to = Camera {
                    zoom: to.zoom.clamp(self.site_min(), SITE_MAX),
                    ..to
                };
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
    #[cfg(test)]
    pub fn end_gesture(&mut self) {
        self.zoom_lock = None;
        self.transition = None;
    }
}
