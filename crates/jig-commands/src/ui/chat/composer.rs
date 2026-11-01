//! The reply box, and keeping the transcript at its end.

use std::rc::Rc;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::h_flex;
use gpui_kit::component::input::{Enter, Textarea, TextareaState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::SubmitHandler;

/// Keep the transcript at its end while it's scrolled there, so a reply
/// shows its last words: Markdown is laid out a frame after it arrives, so
/// a single scroll to the bottom stops short of a long reply. Scrolled up,
/// it stays put. Returns whether there's more below what shows.
pub(super) fn follow_the_end(scroll: &ScrollHandle) -> bool {
    let max = scroll.max_offset().y;
    let below = max + scroll.offset().y;
    if below <= px(4.) {
        scroll.scroll_to_bottom();
        false
    } else {
        true
    }
}

/// Shown while the transcript is scrolled up from its end; jumps there.
pub(super) fn more_below_button(scroll: &ScrollHandle, accent: Hsla, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let scroll = scroll.clone();
    h_flex()
        .absolute()
        .bottom_1()
        .left_0()
        .right_0()
        .justify_center()
        .child(
            h_flex()
                .id("jig-agent-more-below")
                .debug_selector(|| "agent-more-below".into())
                .gap_1()
                .px_2()
                .py_0p5()
                .rounded_full()
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .shadow_sm()
                .text_xs()
                .text_color(accent)
                .cursor_pointer()
                .child("More below ↓")
                .on_click(move |_, window, _| {
                    scroll.scroll_to_bottom();
                    window.refresh();
                }),
        )
}

/// The shared message composer, also used before a conversation is selected.
pub fn message_input(
    input: &Entity<TextareaState>,
    on_submit: impl Fn(&Enter, &mut Window, &mut App) + 'static,
    cx: &App,
) -> Stateful<Div> {
    reply_box(
        input,
        Some(Rc::new(on_submit)),
        crate::ui::surface::agent_accent(cx),
        cx,
    )
}

/// The box the user replies or answers in. The whole box takes the click,
/// not just the text.
pub(super) fn reply_box(
    input: &Entity<TextareaState>,
    on_submit: Option<SubmitHandler>,
    accent: Hsla,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    div()
        .id("jig-agent-reply")
        .debug_selector(|| "agent-reply-box".into())
        .when_some(on_submit, |this, submit| {
            this.capture_action(move |action: &Enter, window, cx| {
                if !action.shift {
                    cx.stop_propagation();
                    submit(action, window, cx);
                }
            })
        })
        .cursor_text()
        .on_mouse_down(MouseButton::Left, {
            let input = input.clone();
            move |_, window, cx| {
                input.update(cx, |input, cx| input.focus(window, cx));
            }
        })
        .px_2()
        .py_1()
        .rounded(px(8.))
        .border_1()
        .border_color(accent.opacity(0.35))
        .bg(theme.foreground.opacity(0.03))
        .child(Textarea::new(input).w_full().appearance(false))
}