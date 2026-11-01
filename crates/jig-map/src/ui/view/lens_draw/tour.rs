//! The tour's guide along the top, and a proposal's buttons.

use super::*;

impl MapView {
    /// The guide along the top while touring: the step showing, with
    /// buttons and the arrow keys to go back and on.
    pub(super) fn render_tour_bar(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let tour = self.lens.tour?;
        let drawing = self.drawing(tour.source)?;
        let step = drawing.steps.get(tour.step)?;
        let agent = agent_accent(cx);
        let count = drawing.steps.len();
        let arrow = |id: &'static str, label: &'static str, delta: i32, enabled: bool| {
            h_flex()
                .id(id)
                .debug_selector(move || id.into())
                .flex_none()
                .h(px(30.))
                .px_3()
                .gap_1()
                .rounded_full()
                .bg(white().opacity(if enabled { 0.22 } else { 0.08 }))
                .text_color(white().opacity(if enabled { 1. } else { 0.45 }))
                .font_weight(FontWeight::SEMIBOLD)
                .when(enabled, |this| {
                    this.cursor_pointer()
                        .hover(|style| style.bg(white().opacity(0.34)))
                })
                .child(label)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |view, _, _, cx| {
                    view.tour_by(delta, cx);
                }))
        };
        Some(
            h_flex()
                .id("map-tour-bar")
                .debug_selector(|| "map-tour-bar".into())
                .absolute()
                .top(px(LIST_TOP))
                .left(px(LIST_WIDTH + 24.))
                .right(px(LIST_WIDTH + 24.))
                .justify_center()
                .child(
                    h_flex()
                        .gap_3()
                        .pl_1p5()
                        .pr_1p5()
                        .py_1p5()
                        .rounded_full()
                        .bg(agent)
                        .text_color(white())
                        .shadow_lg()
                        .child(arrow("map-tour-back", "← Back", -1, tour.step > 0))
                        .child(
                            v_flex()
                                .min_w_0()
                                .items_center()
                                .child(
                                    h_flex()
                                        .gap_2()
                                        .text_base()
                                        .child(
                                            div()
                                                .flex_none()
                                                .opacity(0.85)
                                                .child(format!("{} of {count}", tour.step + 1)),
                                        )
                                        .child(
                                            div()
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .child(step.label.clone()),
                                        ),
                                )
                                .child(div().text_xs().opacity(0.85).child(format!(
                                    "{} · ← → step through · Esc ends",
                                    self.short_name(&step.place)
                                ))),
                        )
                        .child(arrow("map-tour-next", "Next →", 1, tour.step + 1 < count)),
                )
                .into_any_element(),
        )
    }

    /// What to do with a proposal, over its first ghost.
    pub(super) fn render_proposal_buttons(
        &self,
        id: usize,
        first: Rect,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let agent = agent_accent(cx);
        let choices = [
            (ProposalChoice::MakeTask, "Make Task"),
            (ProposalChoice::Implement, "Implement"),
            (ProposalChoice::Adopt, "Adopt"),
            (ProposalChoice::Dismiss, "Dismiss"),
        ];
        h_flex()
            .absolute()
            .left(px(first.x))
            .top(px(first.y - 30.))
            .gap_1()
            .children(choices.into_iter().enumerate().map(|(n, (choice, label))| {
                h_flex()
                    .id(("map-proposal", id * 8 + n))
                    .debug_selector(move || format!("map-proposal-{label}"))
                    .h(px(22.))
                    .px_2()
                    .rounded(px(5.))
                    .text_xs()
                    .when(n == 0, |this| this.bg(agent).text_color(white()))
                    .when(n > 0, |this| {
                        this.bg(theme.background)
                            .border_1()
                            .border_color(theme.border)
                            .text_color(theme.foreground)
                    })
                    .child(label)
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |view, _, _, cx| match choice {
                        // The view adopts it, and says so when it's all done.
                        ProposalChoice::Adopt => {
                            view.adopt(id, None, cx);
                        }
                        choice => cx.emit(MapEvent::Proposal(id, choice)),
                    }))
            }))
            .into_any_element()
    }
}
