//! Shared motion for the floating windows.

use std::time::Duration;

use gpui_kit::*;

/// Fade in while settling a few pixels into place. The animation runs when
/// an element with this `id` first appears; it doesn't replay while the
/// element stays on screen and its content changes.
pub fn pop_in<E: Styled + IntoElement + 'static>(
    element: E,
    id: impl Into<ElementId>,
) -> AnimationElement<E> {
    element.with_animation(
        id,
        Animation::new(Duration::from_millis(140)).with_easing(ease_out_quint()),
        |element, delta| element.opacity(delta).mt(px(6. * (1. - delta))),
    )
}
