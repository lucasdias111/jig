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

use super::scene::Scene;
use super::{Level, MapEvent, MapView, has_plan};
use crate::domain::layout::Rect;
use crate::domain::plan::{Focus, Item};
use crate::domain::site::Kind;
mod cameras;
mod stepping;

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

    /// Where zooming out of this Plan goes to Site.
    pub(super) fn plan_out(&self) -> f32 {
        (self.default_camera().zoom * 0.85).clamp(PLAN_MIN + 0.02, PLAN_OUT)
    }

    pub(super) fn zoom_by(&mut self, factor: f32, around: Point<Pixels>, min: f32, max: f32) {
        let zoom = (self.zoom * factor).clamp(min, max.max(min));
        let (x, y) = self.to_drawing(around);
        let local = around - self.bounds.get().origin;
        self.offset = point(local.x - px(x * zoom), local.y - px(y * zoom));
        self.zoom = zoom;
    }

    /// The middle of the canvas, in the window.
    pub(super) fn middle(&self) -> Point<Pixels> {
        let (width, height) = self.canvas_size();
        self.bounds.get().origin + point(px(width / 2.), px(height / 2.))
    }

    /// For tests: the gesture that changed level has ended.
    #[cfg(any(test, feature = "testing"))]
    pub fn end_gesture(&mut self) {
        self.zoom_lock = None;
        self.transition = None;
    }
}
