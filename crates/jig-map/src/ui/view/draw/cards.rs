//! The boxes on the canvas: frames, and Site's and Plan's cards.

use super::*;
mod plan;

impl MapView {
    pub(super) fn render_frame(
        &self,
        frame: &FrameBox,
        view: View,
        lit: &Lit,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let at = view.point((frame.rect.x, frame.rect.y));
        let (width, height) = (
            view.scaled(frame.rect.width),
            view.scaled(frame.rect.height),
        );
        let selected = lit
            .selected
            .iter()
            .any(|key| key == &format!("site:{}", frame.key));
        let accent = match frame.tint {
            Some(tint) => domain::colour(tint, theme.mode.is_dark()),
            None => look_colour(frame.look, cx),
        };
        let fixed = frame.look.fixed_text();
        // Site's titles fit their title bar, at most the size cards use.
        let bar = frame.header * view.camera.zoom;
        let title_size = if fixed {
            px((bar * 0.55).min(12.))
        } else {
            view.scaled(11.)
        };
        let words = !fixed || bar >= 13.;
        let package = frame.look == Look::Package;
        div()
            .absolute()
            .left(at.x)
            .top(at.y)
            .w(width)
            .h(height)
            .rounded(if fixed { px(8.) } else { view.scaled(10.) })
            .border_1()
            .when(package, |this| this.border_2())
            .border_color(if selected {
                theme.primary
            } else {
                accent.opacity(if package { 0.45 } else { 0.25 })
            })
            .bg(accent.opacity(0.035))
            .opacity(frame.alpha)
            .overflow_hidden()
            .when(words, |this| {
                this.child(
                    h_flex()
                        .h(if fixed { px(bar) } else { view.scaled(24.) })
                        .px(if fixed { px(10.) } else { view.scaled(10.) })
                        .gap_2()
                        .min_w_0()
                        .items_center()
                        .text_size(title_size)
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.foreground.opacity(0.8))
                                .child(frame.title.clone()),
                        )
                        .when(!frame.note.is_empty() && fixed, |this| {
                            this.child(
                                div()
                                    .flex_none()
                                    .text_size(px((bar * 0.48).min(10.5)))
                                    .text_color(theme.muted_foreground)
                                    .child(frame.note.clone()),
                            )
                        }),
                )
            })
            .into_any_element()
    }

    pub(super) fn render_card(
        &self,
        card: &Card,
        view: View,
        live: bool,
        lit: &Lit,
        now: Instant,
        cx: &Context<Self>,
    ) -> AnyElement {
        if card.look.fixed_text() {
            self.render_site_card(card, view, lit, cx)
        } else {
            self.render_plan_card(card, view, live, lit, now, cx)
        }
    }

    /// A Site box: its name, what it's for, how much it holds, at one
    /// size whatever the zoom, as much as fits.
    fn render_site_card(
        &self,
        card: &Card,
        view: View,
        lit: &Lit,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let at = view.point((card.rect.x, card.rect.y));
        let (width, height) = (
            card.rect.width * view.camera.zoom,
            card.rect.height * view.camera.zoom,
        );
        let accent = match card.tint {
            Some(tint) => domain::colour(tint, theme.mode.is_dark()),
            None => look_colour(card.look, cx),
        };
        let selected = lit
            .selected
            .iter()
            .any(|key| key == &format!("site:{}", card.key) || key == &card.key);
        let hovered = lit.hovered.as_deref() == Some(card.key.as_str());
        let words = width >= 46. && height >= 16.;
        let unread = card.look == Look::Unread;
        // As many lines as fit: the title (two at most), what it's for or
        // holds, then how much.
        let lines = ((height - 10.) / 14.).floor().max(1.) as usize;
        let title_lines = if lines >= 3 { 2 } else { 1 };
        let detail_lines = lines.saturating_sub(title_lines + 1);
        div()
            .absolute()
            .left(at.x)
            .top(at.y)
            .w(px(width))
            .h(px(height))
            .overflow_hidden()
            .rounded(px((height / 6.).clamp(2., 7.)))
            .border_1()
            .when(unread, |this| this.border_dashed())
            .border_color(if selected {
                theme.primary
            } else if hovered {
                accent.opacity(0.8)
            } else {
                accent.opacity(0.4)
            })
            .when(selected, |this| this.border_2().shadow_md())
            .bg(blend(
                theme.background,
                accent,
                match (unread, card.tint.is_some()) {
                    (true, _) => 0.04,
                    (false, true) => 0.18,
                    (false, false) => 0.1,
                },
            ))
            .opacity(card.alpha)
            .when(words, |this| {
                this.child(
                    v_flex()
                        .px(px(8.))
                        .py(px(if lines >= 2 { 5. } else { 1. }))
                        .gap(px(1.))
                        .child(
                            div()
                                .line_height(px(14.))
                                .line_clamp(title_lines)
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(if unread {
                                    theme.muted_foreground
                                } else {
                                    theme.foreground
                                })
                                .child(card.title.clone()),
                        )
                        .when(detail_lines > 0 && !card.detail.is_empty(), |this| {
                            this.child(
                                div()
                                    .line_height(px(13.))
                                    .line_clamp(detail_lines)
                                    .text_size(px(10.5))
                                    .text_color(theme.foreground.opacity(0.7))
                                    .child(card.detail.clone()),
                            )
                        })
                        .when(lines >= 2, |this| {
                            this.child(
                                div()
                                    .truncate()
                                    .text_size(px(10.))
                                    .text_color(theme.muted_foreground)
                                    .child(card.note.clone()),
                            )
                        }),
                )
            })
            .into_any_element()
    }

    /// Whether Plan boxes `a` and `b` are joined by a call.
    pub(super) fn joined(&self, a: &str, b: &str) -> bool {
        self.drawn.borrow().arrows.iter().any(|arrow| {
            (arrow.keys.0 == a && arrow.keys.1 == b) || (arrow.keys.0 == b && arrow.keys.1 == a)
        })
    }
}
