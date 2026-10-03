//! The small floating window that shows a command's progress and reply.

use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Lines of removed code shown before the rest is elided.
const REMOVED_LINES: usize = 6;

#[derive(Clone, Debug, PartialEq, IntoElement)]
pub enum Bubble {
    Running {
        label: String,
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
            .rounded_lg()
            .shadow_lg();
        let hint = |text: &'static str| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        };

        match self {
            Bubble::Running { label } => container.child(
                h_flex()
                    .gap_2()
                    .child(Spinner::new().small().color(theme.muted_foreground))
                    .child(div().text_color(theme.muted_foreground).child(label))
                    .child(hint("esc to cancel")),
            ),
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
        }
    }
}
