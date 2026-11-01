//! Drawing the branch picker.

use super::*;

impl Render for BranchPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.rows.len();
        let list = uniform_list(
            "branch-picker-rows",
            rows,
            cx.processor(|this, range: Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .w_full()
        .h(px(rows.min(MAX_VISIBLE_ROWS) as f32 * ROW_HEIGHT));
        let empty = (rows == 0).then(|| {
            div()
                .px_2p5()
                .py_3()
                .text_size(px(13.))
                .text_color(theme.muted_foreground)
                .child(match (self.loading, self.mode) {
                    (true, _) => "Reading branches…",
                    (false, Mode::Create) => "Type a name for the new branch.",
                    (false, Mode::Switch) => "No branches yet. Type a name to create one.",
                })
        });
        let divider = || {
            div()
                .h(px(1.))
                .mx_neg_1p5()
                .bg(theme.foreground.opacity(0.08))
        };
        let panel = jig_commands::surface::panel(cx)
            .flex()
            .flex_col()
            .key_context("JigBranchPicker")
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_up))
            .capture_action(cx.listener(Self::on_down))
            .w(px(WIDTH))
            .p_1p5()
            .child(
                h_flex()
                    .h(px(40.))
                    .pl_2p5()
                    .pr_1()
                    .gap_1()
                    .child(
                        Icon::default()
                            .data(BRANCH_ICON)
                            .size(px(15.))
                            .flex_none()
                            .text_color(theme.primary),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(16.))
                            .child(Input::new(&self.input).appearance(false).cleanable(false)),
                    ),
            )
            .child(divider().mb_1())
            .child(v_flex().children(empty).child(list))
            .child(divider().mt_1())
            .child(h_flex().px_2p5().pt_1p5().pb_0p5().justify_end().child(
                jig_commands::surface::hint(
                    match self.mode {
                        Mode::Switch => "↑↓ navigate · ↩ switch · esc close",
                        Mode::Create => "↩ create and switch · esc close",
                    },
                    cx,
                ),
            ));
        jig_commands::motion::pop_in(panel, "jig-branch-picker")
    }
}
