//! The look shared by the floating windows: a macOS-style panel with a
//! hairline edge, a soft, layered shadow and a faint sheen, like frosted
//! glass. GPUI can't blur what is behind an element, so the panel stays
//! opaque and only looks the part: see-through, the code behind it would
//! compete with its text.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

pub const RADIUS: f32 = 12.;

/// How opaque the editor's surfaces are, from 0 to 1, as the theme last set
/// it from the settings.
pub struct EditorOpacity(pub f32);

impl Global for EditorOpacity {}

/// The background of the editor and the bars around it, letting the
/// window's blur through as far as the settings ask. Canvases shown in the
/// editor's place, such as the Map, draw on it too.
pub fn editor_surface(cx: &App) -> Hsla {
    let opacity = cx.try_global::<EditorOpacity>().map_or(1., |o| o.0);
    cx.theme().background.opacity(opacity)
}

/// A floating panel's surface. Callers add size, padding and content.
/// It takes the mouse: clicks and hovers don't reach the code behind it.
pub fn panel(cx: &App) -> Div {
    let theme = cx.theme();
    // Darker shadows in dark mode, where a light one would read as a glow.
    let depth = if theme.is_dark() { 0.45 } else { 0.12 };
    // Light falls from above: a touch brighter at the top in dark mode, a
    // touch dimmer at the bottom in light mode, where the top is white.
    let (top, bottom) = if theme.is_dark() {
        (lighten(theme.popover, 0.035), theme.popover)
    } else {
        (theme.popover, lighten(theme.popover, -0.02))
    };
    div()
        .occlude()
        .bg(linear_gradient(
            180.,
            linear_color_stop(top, 0.),
            linear_color_stop(bottom, 1.),
        ))
        .text_color(theme.popover_foreground)
        .border_1()
        .border_color(if theme.is_dark() {
            hsla(0., 0., 1., 0.1)
        } else {
            hsla(0., 0., 0., 0.1)
        })
        .rounded(px(RADIUS))
        .shadow(vec![
            // The glass's lit top edge, inside the hairline.
            BoxShadow {
                color: hsla(0., 0., 1., if theme.is_dark() { 0.06 } else { 0.7 }),
                offset: point(px(0.), px(1.)),
                blur_radius: px(0.),
                spread_radius: px(0.),
                inset: true,
            },
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

fn lighten(color: Hsla, amount: f32) -> Hsla {
    Hsla {
        l: (color.l + amount).clamp(0., 1.),
        ..color
    }
}

/// A small, muted line of keyboard hints.
pub fn hint(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(px(11.))
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// The agent (Lucide "bot").
pub const AGENT_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 8V4H8"/><rect width="16" height="12" x="4" y="8" rx="2"/><path d="M2 14h2"/><path d="M20 14h2"/><path d="M15 13v2"/><path d="M9 13v2"/></svg>"#;

/// The agent's colour, used for everything that belongs to it.
pub fn agent_accent(cx: &App) -> Hsla {
    cx.theme().info
}

pub fn agent_icon() -> gpui_kit::component::Icon {
    gpui_kit::component::Icon::default().data(AGENT_ICON)
}

/// A small tag naming the agent, e.g. in the conversation's header.
pub fn agent_tag(cx: &App) -> Div {
    let accent = agent_accent(cx);
    div()
        .flex()
        .items_center()
        .gap_1()
        .px_1p5()
        .py(px(1.))
        .rounded(px(5.))
        .bg(accent.opacity(0.14))
        .text_color(accent)
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .child(agent_icon().size(px(11.)).text_color(accent))
        .child("Agent")
}
