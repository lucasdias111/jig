//! Transcript rows that open: an edit with its diff, and a subagent with
//! what it did and its report.

use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::transcript::file_name;
use super::{CHEVRON, CHEVRON_DOWN, DiffLine, LinkHandler};
use crate::domain::transcript::diff_counts;

/// An edit's row, with how many lines it adds and removes; a click opens
/// its diff below it, in a box of its own that scrolls.
pub(super) fn edit_with_diff(
    index: usize,
    path: String,
    accepted: bool,
    diff: Vec<DiffLine>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let open = window.use_keyed_state(("jig-agent-edit-open", index), cx, |_, _| false);
    let is_open = *open.read(cx);
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let (added, removed) = diff_counts(&diff);
    let row = h_flex()
        .id(("jig-agent-edit", index))
        .debug_selector(move || format!("agent-edit-{index}"))
        .gap_1p5()
        .text_xs()
        .text_color(muted)
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            open.update(cx, |open, cx| {
                *open = !*open;
                cx.notify();
            })
        })
        .child(
            gpui_kit::component::Icon::default()
                .data(if is_open { CHEVRON_DOWN } else { CHEVRON })
                .size(px(11.))
                .text_color(muted),
        )
        .child(if accepted {
            "Edited"
        } else {
            "Rejected an edit to"
        })
        .child(file_name(path, cx))
        .when(added > 0, |this| {
            this.child(div().text_color(theme.success).child(format!("+{added}")))
        })
        .when(removed > 0, |this| {
            this.child(div().text_color(theme.danger).child(format!("−{removed}")))
        });
    v_flex()
        .gap_1()
        .child(row)
        .when(is_open, |this| this.child(diff_view(index, diff, cx)))
        .into_any_element()
}

/// Tallest a subagent's work grows, opened, before it scrolls.
const SUBAGENT_HEIGHT: f32 = 260.;

/// A subagent's row: what it was given, how much it did, and a spinner
/// while it works; a click opens what it did and its report.
pub(super) fn subagent(
    index: usize,
    description: String,
    calls: Vec<String>,
    report: Option<String>,
    on_link: Option<LinkHandler>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let open = window.use_keyed_state(("jig-agent-subagent-open", index), cx, |_, _| false);
    let is_open = *open.read(cx);
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let accent = crate::ui::surface::agent_accent(cx);
    let working = report.is_none();
    let count = match calls.len() {
        0 => String::new(),
        1 => "· 1 tool".into(),
        n => format!("· {n} tools"),
    };
    let row = h_flex()
        .id(("jig-agent-subagent", index))
        .debug_selector(move || format!("agent-subagent-{index}"))
        .gap_1p5()
        .text_xs()
        .text_color(muted)
        .cursor_pointer()
        .hover(|this| this.text_color(theme.foreground))
        .on_click(move |_, _, cx| {
            open.update(cx, |open, cx| {
                *open = !*open;
                cx.notify();
            })
        })
        .child(
            gpui_kit::component::Icon::default()
                .data(if is_open { CHEVRON_DOWN } else { CHEVRON })
                .size(px(11.))
                .text_color(muted),
        )
        .child(div().flex_none().text_color(accent).child("Subagent"))
        .child(div().min_w_0().truncate().child(description))
        .child(div().flex_none().child(count))
        .when(working, |this| {
            this.child(Spinner::new().color(accent).xsmall())
        });
    let body = is_open.then(|| {
        v_flex()
            .id(("jig-agent-subagent-work", index))
            .debug_selector(move || format!("agent-subagent-work-{index}"))
            .max_h(px(SUBAGENT_HEIGHT))
            .overflow_y_scroll()
            .ml(px(5.))
            .pl(px(10.))
            .gap_1()
            .border_l_1()
            .border_color(theme.border)
            .children(
                calls
                    .into_iter()
                    .map(|call| div().text_xs().text_color(muted).child(call)),
            )
            .when(working, |this| {
                this.child(div().text_xs().text_color(muted).child("Working…"))
            })
            .when_some(report, |this, report| {
                this.child(
                    v_flex()
                        .pt_1()
                        .gap_0p5()
                        .child(div().text_xs().text_color(muted).child("Reported"))
                        .child(div().text_sm().child({
                            let view =
                                TextView::markdown(("jig-agent-subagent-report", index), report)
                                    .style(TextViewStyle::default().paragraph_gap(rems(0.4)))
                                    .selectable(true);
                            match on_link.clone() {
                                Some(on_link) => view
                                    .on_link_click(move |link, _, window, cx| {
                                        on_link(link, window, cx)
                                    })
                                    .into_any_element(),
                                None => view.into_any_element(),
                            }
                        })),
                )
            })
    });
    v_flex()
        .gap_1()
        .child(row)
        .children(body)
        .into_any_element()
}

/// Tallest an edit's diff grows before it scrolls.
const DIFF_HEIGHT: f32 = 220.;

/// An edit's lines, removed in red and added in green, with the unchanged
/// lines around them. Long lines scroll sideways rather than wrap.
fn diff_view(index: usize, diff: Vec<DiffLine>, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let line = |sign: &'static str, text: String, bg: Option<Hsla>, color: Hsla| {
        h_flex()
            .min_w_full()
            .px_1p5()
            .gap_1p5()
            .whitespace_nowrap()
            .when_some(bg, |this, bg| this.bg(bg))
            .child(div().flex_none().w(px(8.)).text_color(color).child(sign))
            .child(div().flex_none().text_color(color).child(text))
    };
    div()
        .id(("jig-agent-diff", index))
        .debug_selector(move || format!("agent-diff-{index}"))
        .max_h(px(DIFF_HEIGHT))
        .overflow_scroll()
        .py_1()
        .rounded(px(6.))
        .border_1()
        .border_color(theme.border)
        .bg(theme.foreground.opacity(0.03))
        .font_family(theme.mono_font_family.clone())
        .text_size(px(11.5))
        .child(
            v_flex()
                .min_w_full()
                .children(diff.into_iter().map(|diff_line| match diff_line {
                    DiffLine::Same(text) => line(" ", text, None, theme.muted_foreground),
                    DiffLine::Removed(text) => line(
                        "-",
                        text,
                        Some(theme.danger.opacity(0.12)),
                        theme.foreground,
                    ),
                    DiffLine::Added(text) => line(
                        "+",
                        text,
                        Some(theme.success.opacity(0.14)),
                        theme.foreground,
                    ),
                    DiffLine::Gap => line(" ", "⋯".into(), None, theme.muted_foreground),
                })),
        )
}
