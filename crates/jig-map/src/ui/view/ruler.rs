//! The levels as a ruler in the canvas's top right corner: Code, Plan,
//! Site and Domain marked along it, from in to out, and a needle at how
//! far you are, so zooming shows how close the next level is before it
//! comes. Within a level the needle runs from halfway to the level inside
//! (zoomed in as far as it goes before stepping in) to halfway to the one
//! outside, measured on the zoom's log, so each pinch moves it alike.
//! Clicking a level's name, or the ruler near it, goes there. The code
//! shows it too while a pinch there is on its way out to the Map.

use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::zoom::{self, PLAN_IN, SITE_MAX, SITE_TO_DOMAIN};
use super::{Level, MapEvent, MapView};

/// The levels along the ruler, in to out.
const LEVELS: [&str; 4] = ["Code", "Plan", "Site", "Domain"];
/// How much of the top the ruler takes, from the right edge.
const RULER_WIDTH: f32 = 238.;
const WIDTH: f32 = RULER_WIDTH - 10.;
/// Room either side, so the end names fit.
const INSET: f32 = 22.;
/// Ticks between two levels' marks.
const MINOR: usize = 5;
/// How far down the canvas the ruler takes room; what else sits in that
/// corner goes below it.
pub(super) const RULER_ROOM: f32 = 48.;
/// How much of the way the needle closes on where it's going each frame.
const EASE: f32 = 0.3;

impl MapView {
    /// Where the view is on the ruler: 0 is Code, 3 Domain, a level's
    /// own mark its middle.
    pub fn depth(&self) -> f32 {
        // How far in the zoom is between `out` and `in_`, 0 to 1.
        let inward = |out: f32, in_: f32| {
            if in_ <= out {
                return 0.5;
            }
            ((self.zoom / out).ln() / (in_ / out).ln()).clamp(0., 1.)
        };
        match &self.level {
            Level::Plan(_) => 1.5 - inward(self.plan_out(), PLAN_IN),
            Level::Site => {
                let out = self.site_min() * SITE_TO_DOMAIN;
                2.5 - inward(out, SITE_MAX.max(out * 4.))
            }
            Level::Domain => {
                let in_ = self.site_min() * zoom::DOMAIN_TO_SITE;
                3. - 0.5 * inward(self.domain_min(), in_)
            }
        }
    }

    /// The needle, eased towards `depth` so a level change slides it
    /// along rather than jumping; whether it's still on its way.
    fn needle(&self) -> (f32, bool) {
        let target = self.depth();
        let shown = self.ruler.get().unwrap_or(target);
        let next = shown + (target - shown) * EASE;
        let settled = (target - next).abs() < 0.002;
        let next = if settled { target } else { next };
        self.ruler.set(Some(next));
        (next, !settled)
    }

    pub(super) fn render_ruler(&self, window: &mut Window, cx: &Context<Self>) -> AnyElement {
        let (depth, moving) = self.needle();
        if moving || self.transition.is_some() {
            window.request_animation_frame();
        }
        let view = cx.weak_entity();
        let go: GoTo = Rc::new(move |name, cx| {
            view.update(cx, |view, cx| view.go_to_level(name, cx)).ok();
        });
        ruler(
            depth,
            self.level.name(),
            super::draw::theme::canvas(cx),
            Some(go),
            cx,
        )
    }

    /// The needle starts at `depth`, easing on from there: the Map shown
    /// from a pinch in the code carries on from where the code's ruler was.
    pub fn ruler_from(&self, depth: f32) {
        self.ruler.set(Some(depth));
    }

    /// The ruler's levels: Code goes to the code, the others step there.
    /// Code with nothing picked at Site or Domain goes back to where you
    /// were in the code, rather than into whatever's in the middle.
    pub(super) fn go_to_level(&mut self, name: &str, cx: &mut Context<Self>) {
        match (name, &self.level) {
            ("Code", Level::Domain | Level::Site) if self.selected.is_empty() => {
                cx.emit(MapEvent::Close)
            }
            ("Site", Level::Plan(_)) | ("Domain", Level::Site) => self.step_out(cx),
            ("Domain", Level::Plan(_)) => self.show(Level::Domain, cx),
            ("Site", Level::Domain) => self.step_in(cx),
            ("Plan" | "Code", Level::Domain | Level::Site) => self.step_in(cx),
            ("Code", Level::Plan(_)) => {
                if let Some(focus) = self
                    .plan
                    .focus
                    .or_else(|| self.selected_plan().first().copied())
                {
                    self.plan.focus = Some(focus);
                    self.enter_plan_box(focus, cx);
                } else {
                    cx.emit(MapEvent::Close);
                }
            }
            _ => {}
        }
    }
}

/// Going to a level by its name, from a click on the ruler.
pub type GoTo = Rc<dyn Fn(&'static str, &mut App)>;

/// The ruler with its needle at `depth` (0 is Code, 3 Domain) and the
/// level `current` named in full, on `background`. With `go`, a level's
/// name, or the ruler under it, goes there; without, it only shows.
pub fn ruler(
    depth: f32,
    current: &str,
    background: Hsla,
    go: Option<GoTo>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let span = WIDTH - 2. * INSET;
    let x_of = |depth: f32| INSET + depth / 3. * span;
    let ticks = (0..=3 * MINOR).map(|ix| {
        let major = ix % MINOR == 0;
        let x = INSET + ix as f32 / (3 * MINOR) as f32 * span;
        let near = (ix as f32 / MINOR as f32 - depth).abs();
        div()
            .absolute()
            .left(px(x - 0.5))
            .bottom(px(4.))
            .w(px(1.))
            .h(px(if major { 9. } else { 5. }))
            .bg(theme
                .foreground
                .opacity(if major { 0.45 } else { 0.22 } + (0.3 - near * 0.3).max(0.)))
    });
    // Each name is a column down to the ruler, so a click near a mark
    // goes to its level.
    let names = LEVELS.into_iter().enumerate().map(|(ix, name)| {
        let here = name == current;
        div()
            .id(name)
            .debug_selector(move || format!("map-level-{name}"))
            .absolute()
            .left(px(x_of(ix as f32) - span / 6.))
            .top_0()
            .bottom_0()
            .w(px(span / 3.))
            .pt(px(3.))
            .flex()
            .justify_center()
            .text_xs()
            .when(here, |this| {
                this.text_color(theme.foreground)
                    .font_weight(FontWeight::MEDIUM)
            })
            .when(!here, |this| this.text_color(theme.muted_foreground))
            .child(name)
            .when_some(go.clone(), |this, go| {
                this.cursor_pointer()
                    .when(!here, |this| {
                        this.hover(|style| style.text_color(theme.foreground))
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, _, cx| go(name, cx))
            })
    });
    let line = div()
        .absolute()
        .left(px(INSET))
        .bottom(px(4.))
        .w(px(span))
        .h(px(1.))
        .bg(theme.foreground.opacity(0.3));
    let needle = div()
        .absolute()
        .left(px(x_of(depth) - 1.))
        .bottom(px(4.))
        .w(px(2.))
        .h(px(14.))
        .rounded(px(1.))
        .bg(theme.foreground);
    let cap = div()
        .absolute()
        .left(px(x_of(depth) - 3.5))
        .bottom(px(16.))
        .size(px(7.))
        .rounded_full()
        .bg(theme.foreground);
    div()
        .id("map-ruler")
        .debug_selector(|| "map-ruler".into())
        .absolute()
        .top(px(8.))
        .right(px(10.))
        .w(px(WIDTH))
        .h(px(RULER_ROOM - 12.))
        .rounded(px(7.))
        .bg(background.opacity(0.85))
        .when(go.is_some(), |this| {
            this.tooltip(|window, cx| {
                Tooltip::new("How far out you are: pinch, ⌘+ and ⌘- move along").build(window, cx)
            })
        })
        .child(line)
        .children(ticks)
        .child(needle)
        .child(cap)
        .children(names)
        .into_any_element()
}
