//! Drawing the conversation: its header, the model list, the transcript
//! and, below it, what it's waiting on.

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::composer::{follow_the_end, more_below_button, reply_box};
use super::transcript::transcript_rows;
use super::{
    ChatDrag, ChatResize, ChatStatus, ClickHandler, Conversation, GRIP_ICON, HISTORY_ICON,
    PIN_ICON, TRANSCRIPT_HEIGHT,
};

impl RenderOnce for Conversation {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let empty = self.entries.is_empty();
        let working = matches!(&self.status, ChatStatus::Working { .. });
        let full_page = self.full_page;
        let status = std::mem::replace(&mut self.status, ChatStatus::Waiting);
        let status = self.status_view(status, cx);
        let entries = transcript_rows(
            std::mem::take(&mut self.entries),
            self.on_drawing.clone(),
            self.on_link.clone(),
            window,
            cx,
        );
        let theme = cx.theme();
        let accent = crate::ui::surface::agent_accent(cx);
        let pinned = self.frame.pinned || full_page;
        let on_grab = self.on_grab.filter(|_| !pinned);
        let header_button =
            |id: &'static str, icon: &'static [u8], active: bool, handler: Option<ClickHandler>| {
                let theme = cx.theme();
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .flex_none()
                    .p_0p5()
                    .rounded(px(4.))
                    .cursor_pointer()
                    .hover(|this| this.bg(theme.foreground.opacity(0.08)))
                    .when(active, |this| this.bg(accent.opacity(0.16)))
                    // Not a drag of the header.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when_some(handler, |this, handler| {
                        this.on_click(move |event, window, cx| handler(event, window, cx))
                    })
                    .child(
                        gpui_kit::component::Icon::default()
                            .data(icon)
                            .size(px(13.))
                            .text_color(if active {
                                accent
                            } else {
                                theme.muted_foreground
                            }),
                    )
            };
        let history = header_button("agent-chat-history", HISTORY_ICON, false, self.on_history);
        let pin = header_button("agent-chat-pin", PIN_ICON, pinned, self.on_pin);
        // The header is the handle to move the window by.
        let header = h_flex()
            .id("jig-agent-chat-header")
            .debug_selector(|| "agent-chat-header".into())
            .when(!pinned, |this| this.cursor_grab())
            .py_0p5()
            .when_some(on_grab, |this, on_grab| {
                this.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    on_grab(event, window, cx)
                })
                .on_drag(ChatDrag, |_, _, _, cx| cx.new(|_| EmptyView))
            })
            .gap_2()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(div().truncate().child(self.label))
            .child(div().flex_1())
            .when_some(
                match &self.status {
                    ChatStatus::Working { started, .. } => Some(started.elapsed().as_secs()),
                    _ => None,
                },
                // The spinner animates every frame, so this stays current.
                |this, elapsed| this.child(format!("{elapsed}s")),
            )
            .children(self.models.as_ref().and_then(|models| {
                let label = models.options.get(models.current?)?.clone();
                let on_toggle = self.on_models.clone();
                let theme = cx.theme();
                Some(
                    h_flex()
                        .id("agent-chat-model")
                        .debug_selector(|| "agent-chat-model".into())
                        .flex_none()
                        .max_w(px(160.))
                        .gap_0p5()
                        .px_1()
                        .rounded(px(4.))
                        .cursor_pointer()
                        .hover(|this| this.bg(theme.foreground.opacity(0.08)))
                        .when(models.open, |this| {
                            this.bg(accent.opacity(0.16)).text_color(accent)
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .when_some(on_toggle, |this, on_toggle| {
                            this.on_click(move |event, window, cx| on_toggle(event, window, cx))
                        })
                        .child(div().truncate().child(label))
                        .child("▾"),
                )
            }))
            .child(crate::ui::surface::agent_tag(cx))
            .child(history)
            .when(!full_page, |this| this.child(pin));
        // The models, under the header while their list is open.
        let model_list = self.models.filter(|models| models.open).map(|models| {
            let on_pick = self.on_model.clone();
            v_flex()
                .gap_0p5()
                .p_1()
                .rounded(px(8.))
                .bg(theme.foreground.opacity(0.04))
                .children(
                    models
                        .options
                        .into_iter()
                        .enumerate()
                        .map(|(index, option)| {
                            let on_pick = on_pick.clone();
                            let current = models.current == Some(index);
                            h_flex()
                                .id(("jig-agent-model", index))
                                .debug_selector(move || format!("agent-model-{index}"))
                                .px_2()
                                .py_0p5()
                                .rounded(px(5.))
                                .justify_between()
                                .cursor_pointer()
                                .text_xs()
                                .hover(|this| this.bg(accent.opacity(0.12)))
                                .when(current, |this| this.text_color(accent))
                                .child(option)
                                .when(current, |this| this.child("in use"))
                                .when_some(on_pick, |this, on_pick| {
                                    this.on_click(move |_, window, cx| on_pick(&index, window, cx))
                                })
                        }),
                )
        });

        let transcript = v_flex()
            .id("jig-agent-transcript")
            .map(|this| match (pinned, self.frame.transcript_height) {
                (true, _) => this.size_full(),
                (false, Some(height)) => this.h(height),
                (false, None) => this.max_h(px(TRANSCRIPT_HEIGHT)),
            })
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .gap_2()
            .when(full_page && empty, |this| {
                this.child(
                    v_flex()
                        .flex_1()
                        .justify_center()
                        .items_center()
                        .gap_3()
                        .py_8()
                        .child(
                            crate::ui::surface::agent_icon()
                                .size(px(32.))
                                .text_color(accent),
                        )
                        .child(
                            div()
                                .text_size(px(22.))
                                .child("What would you like to build?"),
                        )
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child("Ask about your project, plan a change, or start coding."),
                        ),
                )
            })
            .children(entries);
        let more_below = follow_the_end(&self.scroll);
        let transcript_bounds = self.frame.transcript_bounds.clone();
        let transcript = div()
            .relative()
            .when(pinned, |this| this.flex_1().min_h_0())
            .child(transcript)
            .vertical_scrollbar(&self.scroll)
            .when(more_below, |this| {
                this.child(more_below_button(&self.scroll, accent, cx))
            })
            .child(
                canvas(
                    move |drawn, _, _| transcript_bounds.set(drawn),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            );

        let panel = if full_page {
            div().text_color(theme.foreground)
        } else {
            crate::ui::surface::panel(cx)
        }
        .flex()
        .flex_col()
        .map(|this| {
            if full_page {
                this.w_full()
            } else {
                this.w(self.frame.width)
            }
        })
        // Pinned, it fills the height its owner gives it.
        .when(pinned, |this| this.h_full())
        .px_3p5()
        .py_2p5()
        .gap_2p5()
        .text_size(px(13.))
        .border_color(accent.opacity(0.45))
        .child(header)
        .children(model_list)
        .child(transcript)
        .child(div().h(px(1.)).bg(theme.foreground.opacity(0.08)))
        .child(status)
        .when(full_page && working, |this| {
            this.child(reply_box(&self.input, self.on_submit.clone(), accent, cx))
        });
        // Remember where it's drawn, outside the panel's slide-in, so a drag
        // starts from there even when it was nudged to fit the window.
        let bounds = self.frame.bounds;
        let grip = div()
            .id("jig-agent-chat-resize")
            .debug_selector(|| "agent-chat-resize".into())
            .absolute()
            .right(px(3.))
            .bottom(px(3.))
            .size(px(14.))
            .cursor_nwse_resize()
            .when_some(self.on_resize_grab, |this, on_grab| {
                this.on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    on_grab(event, window, cx)
                })
            })
            .on_drag(ChatResize, |_, _, _, cx| cx.new(|_| EmptyView))
            .child(
                gpui_kit::component::Icon::default()
                    .data(GRIP_ICON)
                    .size(px(14.))
                    .text_color(theme.muted_foreground.opacity(0.6)),
            );
        div()
            .relative()
            .when(full_page, |this| this.w_full())
            .when(pinned, |this| this.h_full())
            .child(if full_page {
                panel.into_any_element()
            } else {
                crate::ui::motion::pop_in(panel, "jig-agent-chat").into_any_element()
            })
            .child(
                canvas(move |drawn, _, _| bounds.set(drawn), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .when(!pinned, |this| this.child(grip))
    }
}

