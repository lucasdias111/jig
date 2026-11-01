//! Drawing the Map: the header with where you are and its buttons, then
//! each scene showing (the level left fading out under the one coming
//! in): frames behind their boxes, Plan's calls as curves from a box's
//! right to another's left (dashed for guesses and callbacks), Site's as
//! bowed arrows over the boxes, thicker with more calls. Plan's text
//! scales with the view; Site's stays one size, so it reads at any zoom.

use std::time::Instant;

use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::input::Hover;
use super::scene::{Arrow, Card, FrameBox, Look, Scene};
use super::zoom::Camera;
use super::{Expand, Level, MapView, domain};
use crate::domain::layout::{HEADER_HEIGHT, PADDING, ROW_HEIGHT, Rect};
use crate::domain::plan::Focus;
use crate::domain::site::Show;

mod canvas;
mod cards;
mod header;
mod lines;
mod overlays;
pub(super) use lines::{Line, bow_point, flow_label_size, label_spot, leader, obstacles};

const BAR_HEIGHT: f32 = 34.;
/// A link's words kept where they were stay within this many points of
/// it, as zooming in spreads them apart.
const STICKY_REACH: f32 = 220.;
const GRID: f32 = 24.;

/// Where a scene goes on a canvas `size` big, seen by `camera`.
#[derive(Clone, Copy)]
pub(super) struct View {
    pub(super) camera: Camera,
    pub(super) size: (f32, f32),
}

impl View {
    pub(super) fn point(&self, (x, y): (f32, f32)) -> Point<Pixels> {
        point(
            px((x - self.camera.x) * self.camera.zoom + self.size.0 / 2.),
            px((y - self.camera.y) * self.camera.zoom + self.size.1 / 2.),
        )
    }

    pub(super) fn scaled(&self, value: f32) -> Pixels {
        px(value * self.camera.zoom)
    }
}

/// What's lit over a scene: selected, under the pointer, on the paused
/// program's stack, or just fired.
pub(super) struct Lit {
    pub(super) selected: Vec<String>,
    pub(super) hovered: Option<String>,
    path: Vec<String>,
}

use super::input;

/// The arrows' painting, over or under the boxes as the level draws them.
fn paint_holder(paint: AnyElement) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(paint)
        .into_any_element()
}

/// The colour each kind of box is drawn in.
pub(super) fn look_colour(look: Look, cx: &App) -> Hsla {
    let theme = cx.theme();
    match look {
        Look::Function => theme.blue,
        Look::Abstract => theme.cyan,
        Look::External | Look::More | Look::Gone => theme.muted_foreground,
        Look::Folder | Look::Owner => theme.foreground,
        Look::Package => theme.magenta,
        Look::File => theme.blue,
        Look::Unread => theme.muted_foreground,
        Look::Type => theme.green,
        Look::Source => theme.cyan,
        Look::UnreadSource => theme.muted_foreground,
    }
}

/// `a` turned `t` of the way to `b`.
pub(super) fn blend(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b, t) = (Rgba::from(a), Rgba::from(b), t.clamp(0., 1.));
    Hsla::from(Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    })
}

fn paint_grid(
    bounds: Bounds<Pixels>,
    offset: Point<Pixels>,
    zoom: f32,
    colour: Hsla,
    window: &mut Window,
) {
    let step = GRID * zoom;
    if step < 8. {
        return;
    }
    let first = |origin: Pixels, offset: Pixels| {
        let offset = f32::from(offset).rem_euclid(step);
        f32::from(origin) + offset
    };
    let mut x = first(bounds.origin.x, offset.x);
    while x < f32::from(bounds.right()) {
        window.paint_quad(fill(
            Bounds::new(
                point(px(x), bounds.origin.y),
                size(px(1.), bounds.size.height),
            ),
            colour,
        ));
        x += step;
    }
    let mut y = first(bounds.origin.y, offset.y);
    while y < f32::from(bounds.bottom()) {
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x, px(y)),
                size(bounds.size.width, px(1.)),
            ),
            colour,
        ));
        y += step;
    }
}

/// The Map's colours over the theme's.
pub(super) mod theme {
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::*;

    /// Behind everything: the editor's surface, a shade off.
    pub fn canvas(cx: &App) -> Hsla {
        let surface = jig_commands::surface::editor_surface(cx);
        let l = if surface.l > 0.5 {
            surface.l - 0.025
        } else {
            surface.l + 0.02
        };
        Hsla { l, ..surface }
    }

    /// A Plan box's body.
    pub fn card(cx: &App) -> Hsla {
        cx.theme().background
    }
}
