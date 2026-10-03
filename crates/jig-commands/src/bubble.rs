//! The small floating window that shows a command's progress and reply.

use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

/// Lines of proposed code shown before the rest is elided.
const PREVIEW_LINES: usize = 8;

#[derive(Clone, Debug, PartialEq, IntoElement)]
pub enum Bubble {
    Running { label: String },
    Reply { message: String, proposed: String },
    Error(String),
}

impl Bubble {
    pub fn is_running(&self) -> bool {
        matches!(self, Bubble::Running { .. })
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

        match self {
            Bubble::Running { label } => container.child(
                h_flex()
                    .gap_2()
                    .child(Spinner::new().small().color(theme.muted_foreground))
                    .child(div().text_color(theme.muted_foreground).child(label)),
            ),
            Bubble::Error(message) => {
                container.child(div().text_color(theme.danger).child(message))
            }
            Bubble::Reply { message, proposed } => {
                let lines: Vec<&str> = proposed.lines().collect();
                let shown = lines
                    .iter()
                    .take(PREVIEW_LINES)
                    .copied()
                    .collect::<Vec<_>>()
                    .join("\n");
                let hidden = lines.len().saturating_sub(PREVIEW_LINES);
                container
                    .when(!message.is_empty(), |this| this.child(message))
                    .when(!proposed.is_empty(), |this| {
                        this.child(
                            div()
                                .p_2()
                                .rounded_md()
                                .bg(theme.muted)
                                .font_family(theme.mono_font_family.clone())
                                .text_xs()
                                .whitespace_normal()
                                .child(shown),
                        )
                    })
                    .when(hidden > 0, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(format!("… {hidden} more lines")),
                        )
                    })
            }
        }
    }
}
