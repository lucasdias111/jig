//! What the conversation is waiting on, drawn below its transcript: the
//! agent at work, an edit or request to decide on, a question, a sign-in,
//! or the reply box.

use gpui_kit::component::shimmer::ShimmerText;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::composer::reply_box;
use super::transcript::file_name;
use super::{ChatStatus, Conversation, OptionHandler};

/// Tallest a request's command (or other detail) grows before it scrolls.
const REQUEST_DETAIL_HEIGHT: f32 = 140.;

impl Conversation {
    /// The part below the transcript, for `status`.
    pub(super) fn status_view(&self, status: ChatStatus, cx: &App) -> Div {
        let theme = cx.theme();
        let accent = crate::ui::surface::agent_accent(cx);
        let full_page = self.full_page;
        match status {
            ChatStatus::Working { step, .. } => {
                let step = step.get();
                v_flex()
                    .gap_1p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .child(Spinner::new().color(accent))
                            .child(
                                ShimmerText::new(if step.is_empty() {
                                    "Working…".to_string()
                                } else {
                                    format!("{step}…")
                                })
                                .id("jig-agent-step"),
                            ),
                    )
                    .child(crate::ui::bubble::progress_bar(accent, theme.muted))
                    .child(crate::ui::surface::hint("esc to stop the agent", cx))
            }
            ChatStatus::Reviewing { path, removed } => v_flex()
                .gap_2()
                .child(
                    h_flex()
                        .id("jig-agent-edit-file")
                        .debug_selector(|| "agent-edit-file".into())
                        .gap_2()
                        .cursor_pointer()
                        .child("Wants to edit")
                        .child(file_name(path, cx))
                        .when_some(self.on_show_edit.clone(), |this, on_show| {
                            this.on_click(move |event, window, cx| on_show(event, window, cx))
                        }),
                )
                .when(!removed.trim().is_empty(), |this| {
                    this.child(crate::ui::bubble::removed_lines(&removed, cx))
                })
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_1p5()
                        .child(decide_button(0, "Accept", "↩", true, &self.on_decide, cx))
                        .child(decide_button(
                            1,
                            "Accept all",
                            ALWAYS_KEY,
                            false,
                            &self.on_decide,
                            cx,
                        ))
                        .child(decide_button(
                            2,
                            "Reject",
                            "esc",
                            false,
                            &self.on_decide,
                            cx,
                        )),
                ),
            ChatStatus::Asking {
                title,
                detail,
                always,
            } => v_flex()
                .gap_2()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child(format!("{title}?")),
                )
                .when(!detail.trim().is_empty(), |this| {
                    // A long command scrolls in its box, so the buttons
                    // below it stay in reach.
                    this.child(
                        div()
                            .id("jig-agent-request-detail")
                            .debug_selector(|| "agent-request-detail".into())
                            .max_h(px(REQUEST_DETAIL_HEIGHT))
                            .overflow_y_scroll()
                            .px_2()
                            .py_1()
                            .rounded(px(6.))
                            .bg(theme.foreground.opacity(0.05))
                            .font_family(theme.mono_font_family.clone())
                            .text_xs()
                            .child(detail),
                    )
                })
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_1p5()
                        .child(decide_button(0, "Allow", "↩", true, &self.on_decide, cx))
                        .child(decide_button(
                            1,
                            always,
                            ALWAYS_KEY,
                            false,
                            &self.on_decide,
                            cx,
                        ))
                        .child(decide_button(2, "Deny", "esc", false, &self.on_decide, cx)),
                ),
            ChatStatus::SaveTask { title, todos } => v_flex()
                .gap_2()
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .child("Save this plan as a task?"),
                )
                .child(
                    v_flex()
                        .id("jig-agent-task-draft")
                        .debug_selector(|| "agent-task-draft".into())
                        .max_h(px(REQUEST_DETAIL_HEIGHT))
                        .overflow_y_scroll()
                        .gap_0p5()
                        .px_2()
                        .py_1()
                        .rounded(px(6.))
                        .bg(theme.foreground.opacity(0.05))
                        .text_xs()
                        .when(!title.is_empty(), |this| {
                            this.child(div().font_weight(FontWeight::MEDIUM).child(title))
                        })
                        .children(todos.into_iter().enumerate().map(|(index, todo)| {
                            h_flex()
                                .items_start()
                                .gap_1p5()
                                .child(
                                    div()
                                        .text_color(theme.muted_foreground)
                                        .child(format!("{}.", index + 1)),
                                )
                                .child(div().flex_1().min_w_0().child(todo))
                        })),
                )
                .child(
                    h_flex()
                        .flex_wrap()
                        .gap_1p5()
                        .child(decide_button(0, "Save", "↩", true, &self.on_decide, cx))
                        .child(decide_button(
                            2,
                            "Don't save",
                            "esc",
                            false,
                            &self.on_decide,
                            cx,
                        )),
                )
                .child(reply_box(&self.input, self.on_submit.clone(), accent, cx))
                .child(crate::ui::surface::hint(
                    "enter to save, or say what to change · esc not to save",
                    cx,
                )),
            ChatStatus::Question {
                question,
                options,
                position: (at, of),
            } => {
                let on_option = self.on_option.clone();
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .font_weight(FontWeight::MEDIUM)
                            .when(of > 1, |this| this.child(format!("{at}/{of} ")))
                            .child(question),
                    )
                    .child(h_flex().flex_wrap().gap_1p5().children(
                        options.into_iter().enumerate().map(|(index, option)| {
                            let on_option = on_option.clone();
                            div()
                                .id(("jig-agent-option", index))
                                .px_2()
                                .py_0p5()
                                .rounded(px(6.))
                                .border_1()
                                .border_color(accent.opacity(0.35))
                                .cursor_pointer()
                                .hover(|this| this.bg(accent.opacity(0.12)))
                                .text_xs()
                                .child(format!("{} {option}", index + 1))
                                .when_some(on_option, |this, on_option| {
                                    this.on_click(move |_, window, cx| {
                                        on_option(&index, window, cx)
                                    })
                                })
                        }),
                    ))
                    .child(reply_box(&self.input, self.on_submit.clone(), accent, cx))
                    .child(crate::ui::surface::hint(
                        "a number or your own answer · enter to answer · esc to decline",
                        cx,
                    ))
            }
            ChatStatus::SignIn { options, opened } => {
                let on_option = self.on_option.clone();
                v_flex()
                    .gap_2()
                    .child(
                        v_flex()
                            .gap_1()
                            .children(options.into_iter().enumerate().map(|(index, option)| {
                                let on_option = on_option.clone();
                                div()
                                    .id(("jig-agent-sign-in", index))
                                    .px_2()
                                    .py_1()
                                    .rounded(px(6.))
                                    .border_1()
                                    .border_color(accent.opacity(0.35))
                                    .cursor_pointer()
                                    .hover(|this| this.bg(accent.opacity(0.12)))
                                    .text_xs()
                                    .child(option)
                                    .when_some(on_option, |this, on_option| {
                                        this.on_click(move |_, window, cx| {
                                            on_option(&index, window, cx)
                                        })
                                    })
                            })),
                    )
                    .when(opened, |this| {
                        this.child(reply_box(&self.input, self.on_submit.clone(), accent, cx))
                    })
                    .child(crate::ui::surface::hint(
                        if opened {
                            "finish in the terminal, then enter to try again"
                        } else {
                            "click one to sign in · esc to close"
                        },
                        cx,
                    ))
            }
            ChatStatus::Loading => h_flex()
                .gap_2()
                .text_xs()
                .child(Spinner::new().color(accent))
                .child("Loading the conversation…"),
            ChatStatus::Waiting => v_flex()
                .gap_1p5()
                .child(reply_box(&self.input, self.on_submit.clone(), accent, cx))
                .child(crate::ui::surface::hint(
                    if full_page {
                        "enter to send · shift-enter for a new line · esc to stop"
                    } else {
                        "enter to reply · shift-enter for a new line · esc to close"
                    },
                    cx,
                )),
        }
    }
}

/// The key for "always": allowing a kind of request, or accepting every
/// edit, for the rest of the conversation.
const ALWAYS_KEY: &str = if cfg!(target_os = "macos") {
    "⌘↩"
} else {
    "ctrl ↩"
};

/// One of a request's buttons, labelled with its key: 0 says yes, 1 yes
/// for the rest of the conversation, 2 no.
fn decide_button(
    index: usize,
    label: impl Into<SharedString>,
    key: &'static str,
    primary: bool,
    on_decide: &Option<OptionHandler>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let accent = crate::ui::surface::agent_accent(cx);
    let on_decide = on_decide.clone();
    h_flex()
        .id(("jig-agent-decide", index))
        .debug_selector(move || format!("agent-decide-{index}"))
        .gap_1p5()
        .px_2()
        .py_0p5()
        .rounded(px(6.))
        .border_1()
        .border_color(accent.opacity(0.35))
        .when(primary, |this| this.bg(accent.opacity(0.16)))
        .cursor_pointer()
        .hover(|this| this.bg(accent.opacity(0.22)))
        .text_xs()
        .child(div().truncate().child(label.into()))
        .child(div().text_color(theme.muted_foreground).child(key))
        .when_some(on_decide, |this, on_decide| {
            this.on_click(move |_, window, cx| on_decide(&index, window, cx))
        })
}
