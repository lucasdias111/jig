//! The transcript's rows: the user's words, the agent's, what it did and
//! drew, each drawn as [`transcript::rows`] groups them.

use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::details::{edit_with_diff, subagent};
use super::{CHEVRON, CHEVRON_DOWN, ChatEntry, DrawingAction, DrawingHandler, LinkHandler};
use crate::domain::transcript::{self, Row};

/// The transcript's rows, as [`transcript::rows`] groups them, drawn.
pub(super) fn transcript_rows(
    entries: Vec<ChatEntry>,
    on_drawing: Option<DrawingHandler>,
    on_link: Option<LinkHandler>,
    window: &mut Window,
    cx: &mut App,
) -> Vec<AnyElement> {
    let mut rows = Vec::new();
    for row in transcript::rows(entries) {
        let (index, entry) = match row {
            Row::Calls(index, calls) => {
                rows.push(tool_calls(index, calls, window, cx));
                continue;
            }
            Row::Entry(index, entry) => (index, entry),
        };
        rows.push(match entry {
            ChatEntry::Edit {
                path,
                accepted,
                diff,
            } if !diff.is_empty() => edit_with_diff(index, path, accepted, diff, window, cx),
            ChatEntry::Subagent {
                description,
                calls,
                report,
                ..
            } => subagent(
                index,
                description,
                calls,
                report,
                on_link.clone(),
                window,
                cx,
            ),
            ChatEntry::Drawing {
                id,
                summary,
                shown,
                pinned,
                stepped,
                settled,
            } => drawing(
                id,
                summary,
                (shown, pinned, stepped),
                settled,
                on_drawing.clone(),
                cx,
            ),
            ChatEntry::Agent(text) => agent_words(index, text, on_link.clone(), cx),
            entry => render_entry(index, entry, cx),
        });
    }
    rows
}

/// The agent's words, in Markdown; a link clicked goes to `on_link` when
/// there is one.
fn agent_words(index: usize, text: String, on_link: Option<LinkHandler>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let code = StyleRefinement::default()
        .font_family(theme.mono_font_family.clone())
        .text_size(px(12.));
    let view = TextView::markdown(("jig-agent-text", index), text)
        .style(
            TextViewStyle::default()
                .paragraph_gap(rems(0.5))
                .code_block(code),
        )
        .selectable(true);
    match on_link {
        Some(on_link) => view
            .on_link_click(move |link, _, window, cx| on_link(link, window, cx))
            .into_any_element(),
        None => view.into_any_element(),
    }
}

/// A drawing on the Map: what it is, and buttons to show or hide it, pin
/// it and tour it.
fn drawing(
    id: usize,
    summary: String,
    (shown, pinned, stepped): (bool, bool, bool),
    settled: Option<String>,
    on_drawing: Option<DrawingHandler>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let accent = crate::ui::surface::agent_accent(cx);
    let button = |name: &'static str, label: &'static str, action: DrawingAction, on: bool| {
        let handler = on_drawing.clone();
        div()
            .id((name, id))
            .debug_selector(move || format!("agent-drawing-{name}-{id}"))
            .flex_none()
            .px_1p5()
            .rounded(px(4.))
            .text_xs()
            .cursor_pointer()
            .text_color(if on { accent } else { theme.muted_foreground })
            .hover(|this| this.bg(theme.foreground.opacity(0.08)))
            .child(label)
            .when_some(handler, |this, handler| {
                this.on_click(move |_, window, cx| handler(&(id, action), window, cx))
            })
    };
    h_flex()
        .id(("jig-agent-drawing", id))
        .debug_selector(move || format!("agent-drawing-{id}"))
        .gap_1p5()
        .text_xs()
        .text_color(if shown {
            theme.foreground
        } else {
            theme.muted_foreground
        })
        .child(
            crate::ui::surface::agent_icon()
                .size(px(11.))
                .flex_none()
                .text_color(if shown {
                    accent
                } else {
                    theme.muted_foreground
                }),
        )
        .child(div().flex_1().min_w_0().truncate().child(summary))
        .when_some(settled, |this, settled| {
            this.child(
                div()
                    .flex_none()
                    .text_color(theme.muted_foreground)
                    .child(settled),
            )
        })
        .when(stepped && shown, |this| {
            this.child(button("tour", "Tour", DrawingAction::Tour, false))
        })
        .child(button(
            "pin",
            if pinned { "Pinned" } else { "Pin" },
            DrawingAction::Pin,
            pinned,
        ))
        .child(button(
            "toggle",
            if shown { "Hide" } else { "Show" },
            DrawingAction::Toggle,
            shown,
        ))
        .into_any_element()
}

/// Tool calls in a row, folded to their count; a click lists them.
fn tool_calls(index: usize, calls: Vec<String>, window: &mut Window, cx: &mut App) -> AnyElement {
    let open = window.use_keyed_state(("jig-agent-tools-open", index), cx, |_, _| false);
    let is_open = *open.read(cx);
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let summary = h_flex()
        .id(("jig-agent-tools", index))
        .debug_selector(move || format!("agent-tools-{index}"))
        .gap_1()
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
        .child(format!("Used {} tools", calls.len()));
    v_flex()
        .gap_1()
        .child(summary)
        .when(is_open, |this| {
            this.child(
                v_flex()
                    .debug_selector(move || format!("agent-tools-{index}-list"))
                    .ml(px(5.))
                    .pl(px(10.))
                    .gap_1()
                    .border_l_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(muted)
                    .children(calls),
            )
        })
        .into_any_element()
}

fn render_entry(index: usize, entry: ChatEntry, cx: &App) -> AnyElement {
    let theme = cx.theme();
    match entry {
        // The user's words sit on the right, tinted with the agent's colour.
        ChatEntry::User(text) => h_flex()
            .justify_end()
            .child(
                div()
                    .max_w(relative(0.85))
                    .px_2p5()
                    .py_1p5()
                    .rounded(px(9.))
                    .bg(crate::ui::surface::agent_accent(cx).opacity(0.12))
                    .child(text),
            )
            .into_any_element(),
        // The agent may answer at length, in Markdown.
        ChatEntry::Agent(text) => {
            let code = StyleRefinement::default()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(12.));
            TextView::markdown(("jig-agent-text", index), text)
                .style(
                    TextViewStyle::default()
                        .paragraph_gap(rems(0.5))
                        .code_block(code),
                )
                .selectable(true)
                .into_any_element()
        }
        // Drawn by `subagent`, which keeps whether it's open.
        ChatEntry::Subagent { description, .. } => div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(format!("Subagent: {description}"))
            .into_any_element(),
        ChatEntry::Did(text) => div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(text)
            .into_any_element(),
        ChatEntry::Edit { path, accepted, .. } => h_flex()
            .gap_1p5()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(if accepted {
                "Edited"
            } else {
                "Rejected an edit to"
            })
            .child(file_name(path, cx))
            .into_any_element(),
        ChatEntry::Note(text) => div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(text)
            .into_any_element(),
        ChatEntry::Error(text) => div()
            .text_color(theme.danger)
            .child(text)
            .into_any_element(),
        // Drawn by `drawing`, which has its buttons.
        ChatEntry::Drawing { summary, .. } => div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(summary)
            .into_any_element(),
    }
}

/// A project-relative path, set in the code font.
pub(super) fn file_name(path: String, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .px_1p5()
        .rounded(px(4.))
        .bg(theme.foreground.opacity(0.06))
        .font_family(theme.mono_font_family.clone())
        .text_xs()
        .text_color(theme.foreground)
        .child(path)
}
