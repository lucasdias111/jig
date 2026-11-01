//! A Plan's card: a function's name, signature and buttons.

use super::*;

impl MapView {
    /// A Plan box: the function's name in its header, its signature
    /// below, the hubs it calls as badges, and buttons opening the hops
    /// not drawn.
    pub(super) fn render_plan_card(
        &self,
        card: &Card,
        view: View,
        live: bool,
        lit: &Lit,
        now: Instant,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let ix = card.ix;
        let at = view.point((card.rect.x, card.rect.y));
        let scaled = |value: f32| view.scaled(value);
        let accent = look_colour(card.look, cx);
        let selected = lit.selected.contains(&card.key);
        let on_path = lit.path.iter().position(|key| key == &card.key);
        let fired = self.firing.of_box(&card.key, now);
        let dimmed = lit
            .hovered
            .as_ref()
            .is_some_and(|hovered| hovered != &card.key && !self.joined(hovered, &card.key));
        let gone = card.look == Look::Gone;
        let expander = |id: &'static str, label: String, what: Expand, tip: &'static str| {
            div()
                .id((id, ix))
                .debug_selector(move || format!("{id}-{ix}"))
                .flex_none()
                .px(scaled(4.))
                .rounded(scaled(4.))
                .text_size(scaled(10.5))
                .text_color(gpui_kit::white().opacity(0.8))
                .hover(|style| style.bg(gpui_kit::white().opacity(0.18)))
                .child(label)
                .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.expand(ix, what, cx);
                }))
        };
        let [callers, callees, implementations] = card.hidden;
        let buttons = live.then(|| {
            (
                (callers > 0).then(|| {
                    expander(
                        "map-callers",
                        format!("◀ {callers}"),
                        Expand::Callers,
                        "Show its callers",
                    )
                }),
                (callees > 0).then(|| {
                    expander(
                        "map-callees",
                        format!("{callees} ▶"),
                        Expand::Callees,
                        "Show what it calls",
                    )
                }),
                (implementations > 0).then(|| {
                    expander(
                        "map-implementations",
                        format!("{implementations} impl ▶"),
                        Expand::Implementations,
                        "Show its implementations",
                    )
                }),
            )
        });
        let (before, after, impls) = match buttons {
            Some((a, b, c)) => (a, b, c),
            None => (None, None, None),
        };
        let detail = (!card.detail.is_empty()).then(|| {
            div()
                .h(scaled(ROW_HEIGHT))
                .px(scaled(10.))
                .truncate()
                .when(
                    matches!(card.look, Look::Function | Look::Abstract),
                    |this| {
                        this.font_family(theme.mono_font_family.clone())
                            .text_size(scaled(10.5))
                    },
                )
                .text_color(theme.muted_foreground)
                .child(card.detail.clone())
        });
        let badges = (!card.badges.is_empty()).then(|| {
            h_flex()
                .h(scaled(ROW_HEIGHT))
                .px(scaled(10.))
                .gap(scaled(4.))
                .overflow_hidden()
                .children(card.badges.iter().take(4).map(|badge| {
                    div()
                        .flex_none()
                        .px(scaled(4.))
                        .rounded(scaled(4.))
                        .bg(theme.foreground.opacity(0.08))
                        .text_color(theme.muted_foreground)
                        .text_size(scaled(10.))
                        .child(badge.clone())
                }))
        });
        let unknown = card.unknown_target.then(|| {
            div()
                .flex_none()
                .px(scaled(4.))
                .rounded(scaled(4.))
                .border_1()
                .border_dashed()
                .border_color(gpui_kit::white().opacity(0.6))
                .text_size(scaled(10.))
                .child("?")
                .id(("map-unknown", ix))
                .tooltip(|window, cx| {
                    Tooltip::new(
                        "It calls a function it's handed; which one, nothing here can know",
                    )
                    .build(window, cx)
                })
        });
        let checked = card.checked.then(|| {
            div()
                .id(("map-checked", ix))
                .flex_none()
                .text_size(scaled(10.))
                .child("✓")
                .tooltip(|window, cx| {
                    Tooltip::new("Calls checked with the language server").build(window, cx)
                })
        });
        let note = (!card.note.is_empty()).then(|| {
            div()
                .h(scaled(ROW_HEIGHT))
                .px(scaled(10.))
                .truncate()
                .italic()
                .text_size(scaled(10.5))
                .text_color(theme.muted_foreground)
                .child(card.note.clone())
        });
        let body = (detail.is_some() || badges.is_some() || note.is_some()).then(|| {
            v_flex()
                .py(scaled(PADDING))
                .children(detail)
                .children(badges)
                .children(note)
        });
        let border = if selected {
            theme.primary
        } else if on_path.is_some() {
            theme.blue
        } else if card.focus {
            accent
        } else {
            theme.border
        };
        div()
            .id(("map-box", ix))
            .debug_selector(move || format!("map-box-{ix}"))
            .absolute()
            .left(at.x)
            .top(at.y)
            .w(scaled(card.rect.width))
            .h(scaled(card.rect.height))
            .overflow_hidden()
            .rounded(scaled(7.))
            .border_1()
            .when(
                selected || card.focus || on_path.is_some() || fired > 0.,
                |this| this.border_2(),
            )
            .border_color(if fired > 0. {
                blend(border, theme.yellow, fired)
            } else {
                border
            })
            .when(selected || on_path == Some(0), |this| this.shadow_lg())
            .when(!selected, |this| this.shadow_sm())
            .bg(blend(
                match card.tint {
                    Some(tint) => blend(
                        theme::card(cx),
                        domain::colour(tint, theme.mode.is_dark()),
                        0.14,
                    ),
                    None => theme::card(cx),
                },
                theme.yellow,
                fired * 0.25,
            ))
            .opacity(card.alpha * if gone { 0.7 } else { 1. } * if dimmed { 0.5 } else { 1. })
            .text_color(theme.foreground)
            .text_size(scaled(11.5))
            .line_height(scaled(ROW_HEIGHT))
            .child(
                h_flex()
                    .h(scaled(HEADER_HEIGHT))
                    .gap(scaled(4.))
                    .px(scaled(8.))
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(accent.opacity(if gone { 0.4 } else { 0.95 }), 0.),
                        linear_color_stop(
                            Hsla {
                                l: accent.l * 0.6,
                                ..accent
                            }
                            .opacity(if gone { 0.4 } else { 0.92 }),
                            1.,
                        ),
                    ))
                    .font_weight(FontWeight::BOLD)
                    .text_color(gpui_kit::white())
                    .children(before)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(card.title.clone()),
                    )
                    .children(checked)
                    .children(unknown)
                    .children(impls)
                    .children(after),
            )
            .children(body)
            .into_any_element()
    }
}
