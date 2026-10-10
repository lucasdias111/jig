//! The ⌘K palette grows out of the title bar's Jig button as it opens and
//! shrinks back into it as it closes. GPUI can't scale an element, so an
//! empty panel outline does the moving and the palette fades in once it's
//! nearly there.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

use super::Workspace;

const OPEN: Duration = Duration::from_millis(200);
const CLOSE: Duration = Duration::from_millis(150);

/// The palette's width, as `CommandPalette` draws it.
const PALETTE_WIDTH: f32 = 420.;
/// Its height before it has ever been drawn.
const PALETTE_HEIGHT: f32 = 280.;

/// Where the button and the palette were last drawn, and a closing
/// outline still on its way back.
#[derive(Default)]
pub(super) struct PaletteMotion {
    pub(super) button: Rc<Cell<Bounds<Pixels>>>,
    pub(super) palette: Rc<Cell<Bounds<Pixels>>>,
    /// From where to where it shrinks, and which closing it is, so each
    /// one animates afresh.
    closing: Option<(Bounds<Pixels>, Bounds<Pixels>, usize)>,
    closings: usize,
    /// Which opening it is, likewise.
    pub(super) openings: usize,
}

impl PaletteMotion {
    /// Where an opening palette grows from: the button, when it's on screen.
    pub(super) fn grow_from(&self) -> Option<Bounds<Pixels>> {
        let button = self.button.get();
        (button.size.width > px(0.)).then_some(button)
    }
}

impl Workspace {
    /// The palette, measured as it's drawn, fading in while its outline
    /// grows from the button.
    pub(super) fn render_palette(&self, cx: &Context<Self>) -> Option<Vec<AnyElement>> {
        let palette = self.palette.as_ref()?;
        let measured = self.palette_motion.palette.clone();
        let view = div().relative().child(palette.view.clone()).child(
            canvas(move |drawn, _, _| measured.set(drawn), |_, _, _, _| {})
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
        );
        let growing = palette
            .grow_from
            .filter(|_| palette.opened.elapsed() < OPEN);
        let serial = palette.serial;
        let view = match growing {
            Some(_) => view
                .with_animation(
                    ("jig-palette-show", serial),
                    Animation::new(OPEN),
                    |view, delta| view.opacity(((delta - 0.45) / 0.55).clamp(0., 1.)),
                )
                .into_any_element(),
            None => view.into_any_element(),
        };
        let mut shown = vec![
            deferred(
                anchored()
                    .position(palette.anchor)
                    .snap_to_window_with_margin(px(8.))
                    .child(view),
            )
            .into_any_element(),
        ];
        if let Some(from) = growing {
            let last = self.palette_motion.palette.get().size;
            let height = if last.height > px(0.) {
                last.height
            } else {
                px(PALETTE_HEIGHT)
            };
            let to = Bounds::new(palette.anchor, size(px(PALETTE_WIDTH), height));
            shown.push(outline(
                ("jig-palette-grow", serial),
                from,
                to,
                OPEN,
                true,
                cx,
            ));
        }
        Some(shown)
    }

    /// The outline shrinking back into the button after the palette closed.
    pub(super) fn render_palette_closing(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (from, to, serial) = self.palette_motion.closing?;
        Some(outline(
            ("jig-palette-shrink", serial),
            from,
            to,
            CLOSE,
            false,
            cx,
        ))
    }

    /// The palette just closed: shrink it back into the button, if both were
    /// on screen.
    pub(super) fn shrink_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(to), from) = (
            self.palette_motion.grow_from(),
            self.palette_motion.palette.get(),
        ) else {
            return;
        };
        if from.size.width <= px(0.) {
            return;
        }
        self.palette_motion.closings += 1;
        let serial = self.palette_motion.closings;
        self.palette_motion.closing = Some((from, to, serial));
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(CLOSE).await;
            this.update(cx, |this, cx| {
                if this
                    .palette_motion
                    .closing
                    .is_some_and(|(_, _, shown)| shown == serial)
                {
                    this.palette_motion.closing = None;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Redraw once the palette's outline has finished growing, so it's
    /// taken away.
    pub(super) fn after_palette_grows(&self, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(OPEN).await;
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
    }
}

/// An empty panel moving from `from` to `to`: in full while it grows,
/// fading as it arrives; fading as it shrinks. It takes no clicks.
fn outline(
    id: impl Into<ElementId>,
    from: Bounds<Pixels>,
    to: Bounds<Pixels>,
    duration: Duration,
    growing: bool,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let border = if theme.is_dark() {
        hsla(0., 0., 1., 0.12)
    } else {
        hsla(0., 0., 0., 0.12)
    };
    let shape = div()
        .absolute()
        .bg(theme.popover)
        .border_1()
        .border_color(border)
        .rounded(px(jig_commands::surface::RADIUS))
        .shadow_lg();
    deferred(shape.with_animation(
        id,
        Animation::new(duration).with_easing(ease_out_quint()),
        move |shape, delta| {
            let at = |a: Pixels, b: Pixels| a + (b - a) * delta;
            let opacity = if growing {
                1. - ((delta - 0.7) / 0.3).clamp(0., 1.)
            } else {
                1. - delta * 0.8
            };
            shape
                .left(at(from.origin.x, to.origin.x))
                .top(at(from.origin.y, to.origin.y))
                .w(at(from.size.width, to.size.width))
                .h(at(from.size.height, to.size.height))
                .opacity(opacity)
        },
    ))
    .into_any_element()
}
