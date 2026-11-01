//! The small floating window that shows the agent's progress, an edit up
//! for review, a note or an error.

use std::time::{Duration, Instant};

use gpui_kit::component::shimmer::ShimmerText;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::domain::transcript::LiveStep;

/// Lines of removed code shown before the rest is elided.
const REMOVED_LINES: usize = 6;
/// The widest the bubble gets, to find it room beside the code.
pub const MAX_WIDTH: f32 = 520.;

#[derive(Clone, Debug, PartialEq, IntoElement)]
pub enum Bubble {
    /// Waiting for the agent. `started` drives the elapsed-time counter.
    Running {
        label: String,
        started: Instant,
        /// Project files that went with the request, e.g. "AGENTS.md".
        context: Vec<String>,
        /// What the agent is doing, updated from its thread.
        step: LiveStep,
    },
    /// A change is in the buffer awaiting accept or reject. `removed` is the
    /// code it replaced.
    /// `message` names the file. The agent carries on once it's settled.
    Preview {
        message: String,
        removed: String,
    },
    /// A note, e.g. why nothing can run yet.
    Message(String),
    /// The agent finished; its closing sentence.
    AgentDone(String),
    Error(String),
}

impl Bubble {
    pub fn is_running(&self) -> bool {
        matches!(self, Bubble::Running { .. })
    }

    pub fn is_preview(&self) -> bool {
        matches!(self, Bubble::Preview { .. })
    }
}

impl RenderOnce for Bubble {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let container = crate::ui::surface::panel(cx)
            .flex()
            .flex_col()
            .max_w(px(MAX_WIDTH))
            .min_w(px(240.))
            .px_3p5()
            .py_2p5()
            .gap_2()
            .text_size(px(13.));
        let hint = |text: &'static str| crate::ui::surface::hint(text, cx);

        let bubble = match self {
            Bubble::Running {
                label,
                started,
                context,
                step,
            } => {
                let accent = crate::ui::surface::agent_accent(cx);
                let elapsed = started.elapsed().as_secs();
                let step = step.get();
                container
                    .border_color(accent.opacity(0.45))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Spinner::new().color(accent))
                            .child(
                                ShimmerText::new(format!("{label}…"))
                                    .id("jig-running-label")
                                    .font_weight(FontWeight::MEDIUM),
                            )
                            .child(div().flex_1())
                            .child(crate::ui::surface::agent_tag(cx))
                            // The spinner animates every frame, so this stays current.
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{elapsed}s")),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(if step.is_empty() {
                                "Working".to_string()
                            } else {
                                step
                            }),
                    )
                    .child(progress_bar(accent, theme.muted))
                    .child({
                        let cancel = "esc to stop the agent";
                        crate::ui::surface::hint(
                            if context.is_empty() {
                                cancel.to_string()
                            } else {
                                format!("with {} · {cancel}", context.join(", "))
                            },
                            cx,
                        )
                    })
            }
            Bubble::Error(message) => {
                container.child(div().text_color(theme.danger).child(message))
            }
            Bubble::Message(message) => container.child(message).child(hint("esc to close")),
            Bubble::AgentDone(message) => {
                let accent = crate::ui::surface::agent_accent(cx);
                container
                    .border_color(accent.opacity(0.45))
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("Agent finished")
                            .child(div().flex_1())
                            .child(crate::ui::surface::agent_tag(cx)),
                    )
                    .child(message)
                    .child(hint("esc to close"))
            }
            Bubble::Preview { message, removed } => {
                let accent = crate::ui::surface::agent_accent(cx);
                container
                    // Which file, so an edit in another tab isn't a surprise.
                    .border_color(accent.opacity(0.45))
                    .child(
                        h_flex()
                            .gap_2()
                            .child("Wants to edit")
                            .child(
                                div()
                                    .px_1p5()
                                    .rounded(px(4.))
                                    .bg(theme.foreground.opacity(0.06))
                                    .font_family(theme.mono_font_family.clone())
                                    .text_xs()
                                    .child(message.clone()),
                            )
                            .child(div().flex_1())
                            .child(crate::ui::surface::agent_tag(cx)),
                    )
                    .when(!removed.trim().is_empty(), |this| {
                        this.child(removed_lines(&removed, cx))
                    })
                    .child(hint(
                        "tab or enter to accept · esc to reject · the agent then carries on",
                    ))
            }
        };
        crate::ui::motion::pop_in(bubble, "jig-bubble")
    }
}

/// The code a change removes, struck through, cut after a few lines.
pub(crate) fn removed_lines(removed: &str, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let lines: Vec<&str> = removed.lines().collect();
    let shown = lines
        .iter()
        .take(REMOVED_LINES)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    let hidden = lines.len().saturating_sub(REMOVED_LINES);
    v_flex()
        .px_2p5()
        .py_2()
        .rounded(px(7.))
        .bg(theme.danger.opacity(0.08))
        .font_family(theme.mono_font_family.clone())
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(div().line_through().child(shown))
        .when(hidden > 0, |this| {
            this.child(format!("… {hidden} more lines removed"))
        })
}

/// An indeterminate progress bar: a segment sliding across a track.
pub(crate) fn progress_bar(color: Hsla, track: Hsla) -> impl IntoElement {
    div()
        .relative()
        .w_full()
        .h(px(3.))
        .rounded_full()
        .overflow_hidden()
        .bg(track)
        .child(
            div()
                .absolute()
                .top_0()
                .h_full()
                .w(relative(0.35))
                .rounded_full()
                .bg(color)
                .with_animation(
                    "jig-progress",
                    Animation::new(Duration::from_millis(1400))
                        .repeat()
                        .with_easing(ease_in_out),
                    |bar, delta| bar.left(relative(-0.35 + 1.35 * delta)),
                ),
        )
}
