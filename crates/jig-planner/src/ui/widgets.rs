//! The planner's small pieces, shared by the list and the form: icons,
//! icon buttons, one-line fields and checkboxes.

use gpui_kit::component::input::Input;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Lucide "list-todo", for Planner navigation.
pub const TASKS_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="5" width="6" height="6" rx="1"/><path d="m3 17 2 2 4-4"/><path d="M13 6h8"/><path d="M13 12h8"/><path d="M13 18h8"/></svg>"#;
/// Lucide "chevron-right".
pub const CHEVRON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>"#;
/// Lucide "chevron-down".
pub const CHEVRON_DOWN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;
/// Lucide "pencil".
pub const PENCIL: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/></svg>"#;
/// Lucide "trash-2".
pub const TRASH: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/><line x1="10" x2="10" y1="11" y2="17"/><line x1="14" x2="14" y1="11" y2="17"/></svg>"#;
/// Lucide "x".
pub const CROSS: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"#;
pub const CHECK: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>"#;

/// An icon button the size of a row's.
pub fn small_button(
    id: impl Into<ElementId>,
    icon: &'static [u8],
    tooltip: &'static str,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    div()
        .id(id)
        .flex_none()
        .size(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .text_color(theme.muted_foreground)
        .hover(|s| {
            s.bg(theme.foreground.opacity(0.1))
                .text_color(theme.foreground)
        })
        .child(Icon::default().data(icon).size(px(12.)))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
}

/// A one-line field in the sidebar's style.
pub fn field(input: Input, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .w_full()
        .h(px(28.))
        .px_2()
        .flex()
        .items_center()
        .rounded(px(7.))
        .border_1()
        .border_color(theme.foreground.opacity(0.12))
        .bg(theme.foreground.opacity(0.03))
        .text_size(px(12.5))
        .child(input.w_full())
}

/// A small checkbox, like the Git view's.
pub fn checkbox(done: bool, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .flex_none()
        .size(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(if done {
            theme.primary
        } else {
            theme.foreground.opacity(0.25)
        })
        .map(|this| {
            if done {
                this.bg(theme.primary).child(
                    Icon::default()
                        .data(CHECK)
                        .size(px(10.))
                        .text_color(theme.primary_foreground),
                )
            } else {
                this.bg(theme.foreground.opacity(0.03))
            }
        })
}
