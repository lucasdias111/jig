//! The small floating window that shows a command's progress and reply.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use gpui_kit::component::shimmer::ShimmerText;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Lines of removed code shown before the rest is elided.
const REMOVED_LINES: usize = 6;

#[derive(Clone, Debug, PartialEq, IntoElement)]
pub enum Bubble {
    /// Waiting for the model. `started` drives the elapsed-time counter.
    Running {
        label: String,
        started: Instant,
        /// Project rules (`JIG.md`) went with the request.
        with_rules: bool,
        /// What an exploring command is looking at, updated from the
        /// request's thread. `None` for commands that don't explore.
        step: Option<LiveStep>,
    },
    /// A change is in the buffer awaiting accept or reject. `removed` is the
    /// code it replaced.
    Preview {
        message: String,
        removed: String,
    },
    /// A reply that changes nothing, e.g. an explanation.
    Message(String),
    Error(String),
}

/// Text shared with a background request, read on every frame.
#[derive(Clone, Default)]
pub struct LiveStep(Arc<Mutex<String>>);

impl LiveStep {
    pub fn set(&self, text: String) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = text;
    }

    pub fn get(&self) -> String {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl PartialEq for LiveStep {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for LiveStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("LiveStep").field(&self.get()).finish()
    }
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
        let container = v_flex()
            .max_w(px(520.))
            .min_w(px(220.))
            .px_3()
            .py_2()
            .gap_1p5()
            .text_sm()
            .bg(theme.popover)
            .text_color(theme.popover_foreground)
            .border_1()
            .border_color(theme.border)
            .rounded(px(10.))
            .shadow_lg();
        let hint = |text: &'static str| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        };

        let bubble = match self {
            Bubble::Running {
                label,
                started,
                with_rules,
                step,
            } => {
                let accent = theme.primary;
                let elapsed = started.elapsed().as_secs();
                let step = step.map(|step| step.get());
                container
                    .border_color(accent.opacity(0.6))
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
                            // The spinner animates every frame, so this stays current.
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{elapsed}s")),
                            ),
                    )
                    .when_some(step, |this, step| {
                        let text = if step.is_empty() {
                            "Exploring the project".to_string()
                        } else {
                            step
                        };
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .truncate()
                                .child(text),
                        )
                    })
                    .child(progress_bar(accent, theme.muted))
                    .child(hint(if with_rules {
                        "with JIG.md · esc to cancel"
                    } else {
                        "esc to cancel"
                    }))
            }
            Bubble::Error(message) => {
                container.child(div().text_color(theme.danger).child(message))
            }
            Bubble::Message(message) => container.child(message).child(hint("esc to close")),
            Bubble::Preview { message, removed } => {
                let lines: Vec<&str> = removed.lines().collect();
                let shown = lines
                    .iter()
                    .take(REMOVED_LINES)
                    .copied()
                    .collect::<Vec<_>>()
                    .join("\n");
                let hidden = lines.len().saturating_sub(REMOVED_LINES);
                container
                    .when(!message.is_empty(), |this| this.child(message))
                    .when(!removed.trim().is_empty(), |this| {
                        this.child(
                            v_flex()
                                .p_2()
                                .rounded_md()
                                .bg(theme.danger.opacity(0.08))
                                .font_family(theme.mono_font_family.clone())
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(div().line_through().child(shown))
                                .when(hidden > 0, |this| {
                                    this.child(format!("… {hidden} more lines removed"))
                                }),
                        )
                    })
                    .child(hint("tab or enter to accept · esc to reject"))
            }
        };
        crate::motion::pop_in(bubble, "jig-bubble")
    }
}

/// An indeterminate progress bar: a segment sliding across a track.
fn progress_bar(color: Hsla, track: Hsla) -> impl IntoElement {
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
