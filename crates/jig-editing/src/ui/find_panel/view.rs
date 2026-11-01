//! Drawing the find panel: its fields, buttons and match count.

use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, h_flex, v_flex};

use super::*;

impl FindPanel {
    /// A small icon button that leaves the keyboard where it is.
    fn icon_button(
        &self,
        id: &'static str,
        icon: &'static [u8],
        tooltip: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let theme = cx.theme();
        div()
            .id(id)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(24.))
            .rounded(px(6.))
            .child(
                Icon::default()
                    .data(icon)
                    .size(px(14.))
                    .text_color(if enabled {
                        theme.muted_foreground
                    } else {
                        theme.muted_foreground.opacity(0.4)
                    }),
            )
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|s| s.bg(theme.foreground.opacity(0.06)))
                    .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            })
            .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .into_any_element()
    }

    fn text_button(
        &self,
        id: &'static str,
        label: &'static str,
        tooltip: &'static str,
        enabled: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        let theme = cx.theme();
        div()
            .id(id)
            .flex_none()
            .h(px(24.))
            .px_2()
            .flex()
            .items_center()
            .rounded(px(6.))
            .text_size(px(12.))
            .border_1()
            .border_color(theme.foreground.opacity(0.1))
            .text_color(if enabled {
                theme.popover_foreground
            } else {
                theme.muted_foreground.opacity(0.5)
            })
            .child(label)
            .when(enabled, |this| {
                this.cursor_pointer()
                    .hover(|s| s.bg(theme.foreground.opacity(0.06)))
                    .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            })
            .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .into_any_element()
    }

    fn render_match_case(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let on = self.match_case;
        div()
            .id("find-match-case")
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .h(px(22.))
            .min_w(px(26.))
            .px_1()
            .rounded(px(5.))
            .text_size(px(11.5))
            .font_family(theme.mono_font_family.clone())
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .border_1()
            .when(on, |this| {
                this.bg(theme.primary.opacity(0.16))
                    .text_color(theme.primary)
                    .border_color(theme.primary.opacity(0.45))
            })
            .when(!on, |this| {
                this.text_color(theme.muted_foreground)
                    .border_color(transparent_black())
                    .hover(|s| s.bg(theme.foreground.opacity(0.06)))
            })
            .child("Cc")
            .tooltip(|window, cx| Tooltip::new("Match Case (⌥C)").build(window, cx))
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.on_toggle_match_case(&ToggleMatchCase, window, cx)
            }))
            .into_any_element()
    }
}

impl Render for FindPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_matches = !self.editor.read(cx).search_session().matcher.is_empty();
        let can_replace = self.can_replace(cx);
        let status = self.status(cx);
        let toggle = self.icon_button(
            "find-toggle-replace",
            if self.replacing {
                CHEVRON_DOWN
            } else {
                CHEVRON_RIGHT
            },
            "Toggle Replace",
            true,
            cx,
            |this, window, cx| this.toggle_replacing(window, cx),
        );
        let match_case = self.render_match_case(cx);
        let previous = self.icon_button(
            "find-previous",
            ARROW_UP,
            "Previous Match (⇧↩)",
            has_matches,
            cx,
            |this, _, cx| this.previous(cx),
        );
        let next = self.icon_button(
            "find-next",
            ARROW_DOWN,
            "Next Match (↩)",
            has_matches,
            cx,
            |this, _, cx| this.next(cx),
        );
        let close = self.icon_button("find-close", CLOSE, "Close (esc)", true, cx, |_, _, cx| {
            cx.emit(FindPanelEvent::Dismissed)
        });
        let replace_row = self.replacing.then(|| {
            let enabled = can_replace && has_matches;
            let replace = self.text_button(
                "find-replace",
                "Replace",
                "Replace (↩)",
                enabled,
                cx,
                |this, window, cx| this.replace_one(window, cx),
            );
            let replace_all = self.text_button(
                "find-replace-all",
                "All",
                "Replace All (⌥⌘↩)",
                enabled,
                cx,
                |this, window, cx| {
                    this.replace_all(window, cx);
                },
            );
            h_flex()
                .gap_1()
                .pl(px(28.))
                .child(field(&self.replacement, cx))
                .child(replace)
                .child(replace_all)
        });
        let theme = cx.theme();
        let read_only = (self.replacing && !can_replace).then(|| {
            div()
                .pl(px(32.))
                .pb_0p5()
                .child(jig_commands::surface::hint(
                    "Read-only while a change is under review. Accept or reject it first.",
                    cx,
                ))
        });

        let panel = jig_commands::surface::panel(cx)
            .track_focus(&self.focus)
            .key_context(CONTEXT)
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_tab))
            .capture_action(cx.listener(Self::on_shift_tab))
            .on_action(cx.listener(Self::on_replace_all))
            .on_action(cx.listener(Self::on_toggle_match_case))
            .w(px(WIDTH))
            .p_1()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_1()
                            .child(toggle)
                            .child(field(&self.query, cx).child(match_case))
                            .child(
                                div()
                                    .flex_none()
                                    .min_w(px(64.))
                                    .text_size(px(11.5))
                                    .text_color(if has_matches || status.is_empty() {
                                        theme.muted_foreground
                                    } else {
                                        theme.danger
                                    })
                                    .child(status),
                            )
                            .child(previous)
                            .child(next)
                            .child(close),
                    )
                    .children(replace_row)
                    .children(read_only),
            );
        jig_commands::motion::pop_in(panel, "jig-find-panel")
    }
}

/// One of the panel's two text fields, in a faint well.
fn field(input: &Entity<InputState>, cx: &App) -> Div {
    let theme = cx.theme();
    h_flex()
        .flex_1()
        .min_w_0()
        .h(px(28.))
        .pl_2()
        .pr_0p5()
        .gap_1()
        .rounded(px(7.))
        .bg(theme.foreground.opacity(0.05))
        .text_size(px(13.))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(Input::new(input).appearance(false).cleanable(false)),
        )
}
