//! Drawing the hover and the signature.

use super::*;

impl CodeHover {
    fn render_hover(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (range, markdown, problems) = self.shown(cx)?;
        let state = self.editor.read(cx);
        let at = state
            .range_to_bounds(&range)
            .or_else(|| state.range_to_bounds(&(range.start..range.start)))?;
        // Below the name, or above it low in the window.
        let above = at.top() > window.viewport_size().height * 0.6;
        let (position, corner) = if above {
            (
                point(at.left() - px(10.), at.top() - px(4.)),
                Anchor::BottomLeft,
            )
        } else {
            (
                point(at.left() - px(10.), at.bottom() + px(4.)),
                Anchor::TopLeft,
            )
        };
        if self.hidden(cx) {
            return None;
        }
        if let Some(first) = problems.first() {
            self.fixes
                .update(cx, |fixes, cx| fixes.offer_for(first, cx));
        }
        let theme = cx.theme();
        let has_problems = !problems.is_empty();
        let docs = markdown.map(|markdown| {
            let code = StyleRefinement::default()
                .bg(theme.transparent)
                .p_0()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(12.));
            TextView::markdown("hover-docs", markdown)
                .style(
                    TextViewStyle::default()
                        .paragraph_gap(rems(0.5))
                        .code_block(code),
                )
                .selectable(true)
        });
        let panel = jig_commands::surface::panel(cx)
            .id("code-hover")
            .debug_selector(|| "code-hover".into())
            .max_w(px(PANEL_MAX_WIDTH))
            .max_h(px(HOVER_MAX_HEIGHT))
            .overflow_y_scroll()
            .px_2p5()
            .py_1p5()
            .text_size(px(12.5))
            .on_hover(cx.listener(|this, held: &bool, _, cx| this.set_held(*held, cx)))
            .child(
                v_flex()
                    .gap_1p5()
                    .children(problems.iter().map(|entry| problem_row(entry, cx)))
                    .when(has_problems, |this| this.child(self.fixes.clone()))
                    .when(has_problems && docs.is_some(), |this| {
                        this.child(div().h(px(1.)).bg(theme.border))
                    })
                    .children(docs),
            );
        Some(
            deferred(
                anchored()
                    .anchor(corner)
                    .position(position)
                    .snap_to_window_with_margin(px(8.))
                    .child(panel),
            )
            .with_priority(PANEL_PRIORITY)
            .into_any_element(),
        )
    }

    fn render_signature(&self, cx: &App) -> Option<AnyElement> {
        let call = self.call.as_ref()?;
        let state = self.editor.read(cx);
        let cursor = state.cursor();
        let at = state.range_to_bounds(&(cursor..cursor))?;
        let position = point(at.left() - px(10.), at.top() - px(4.));
        if self.hidden(cx) {
            return None;
        }
        let theme = cx.theme();
        let signature = &call.signature;
        let emphasis = HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            color: Some(theme.link),
            ..Default::default()
        };
        let label = StyledText::new(signature.label.clone())
            .with_highlights(signature.active.clone().map(|range| (range, emphasis)));
        let panel = jig_commands::surface::panel(cx)
            .id("signature-help")
            .debug_selector(|| "signature-help".into())
            .max_w(px(PANEL_MAX_WIDTH))
            .px_2p5()
            .py_1p5()
            .text_size(px(12.5))
            .child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .font_family(theme.mono_font_family.clone())
                            .child(label),
                    )
                    .when_some(signature.overload, |this, (n, of)| {
                        this.child(
                            div()
                                .flex_none()
                                .text_size(px(11.))
                                .text_color(theme.muted_foreground)
                                .child(format!("{n}/{of}")),
                        )
                    }),
            )
            .when_some(signature.documentation.clone(), |this, docs| {
                this.child(
                    div()
                        .mt_0p5()
                        .text_size(px(11.5))
                        .text_color(theme.muted_foreground)
                        .child(docs),
                )
            });
        Some(
            deferred(
                anchored()
                    .anchor(Anchor::BottomLeft)
                    .position(position)
                    .snap_to_window_with_margin(px(8.))
                    .child(panel),
            )
            .with_priority(PANEL_PRIORITY)
            .into_any_element(),
        )
    }
}

impl Render for CodeHover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .children(self.render_hover(window, cx))
            .children(self.render_signature(cx))
    }
}
