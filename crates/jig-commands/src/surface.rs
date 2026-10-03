//! The look shared by the floating windows: a macOS-style panel with a
//! hairline edge and a soft, layered shadow.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

pub const RADIUS: f32 = 12.;

/// A floating panel's surface. Callers add size, padding and content.
/// It takes the mouse: clicks and hovers don't reach the code behind it.
pub fn panel(cx: &App) -> Div {
    let theme = cx.theme();
    // Darker shadows in dark mode, where a light one would read as a glow.
    let depth = if theme.is_dark() { 0.45 } else { 0.12 };
    div()
        .occlude()
        .bg(theme.popover)
        .text_color(theme.popover_foreground)
        .border_1()
        .border_color(if theme.is_dark() {
            hsla(0., 0., 1., 0.1)
        } else {
            hsla(0., 0., 0., 0.1)
        })
        .rounded(px(RADIUS))
        .shadow(vec![
            BoxShadow {
                color: hsla(0., 0., 0., depth * 0.5),
                offset: point(px(0.), px(1.)),
                blur_radius: px(3.),
                spread_radius: px(0.),
                inset: false,
            },
            BoxShadow {
                color: hsla(0., 0., 0., depth),
                offset: point(px(0.), px(12.)),
                blur_radius: px(32.),
                spread_radius: px(-4.),
                inset: false,
            },
        ])
}

/// A small, muted line of keyboard hints.
pub fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(px(11.))
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}
