//! The agent's drawings over the canvas, and arrows to those out of view.

use super::*;

/// Where on the canvas edge the arrow to a box out of view goes, and
/// which way it points.
pub(in crate::ui::view) fn edge_spot(
    on_screen: Rect,
    (width, height): (f32, f32),
) -> (f32, f32, &'static str) {
    let (cx0, cy0) = (width / 2., height / 2.);
    let (tx, ty) = on_screen.centre();
    let (dx, dy) = (tx - cx0, ty - cy0);
    let scale =
        ((width / 2. - EDGE) / dx.abs().max(0.001)).min((height / 2. - EDGE) / dy.abs().max(0.001));
    let arrow = match (dx.abs() > dy.abs(), dx > 0., dy > 0.) {
        (true, true, _) => "→",
        (true, false, _) => "←",
        (false, _, true) => "↓",
        (false, _, false) => "↑",
    };
    (cx0 + dx * scale, cy0 + dy * scale, arrow)
}

/// Words under a box, at most a few lines.
pub(in crate::ui::view) fn capped(words: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = words.iter().take(MOST_WORDS).cloned().collect();
    if words.len() > MOST_WORDS {
        lines.push(format!("+{} more", words.len() - MOST_WORDS));
    }
    lines
}

impl MapView {
    /// Everything the agent drew, over the scene seen through `view`.
    pub(in crate::ui::view) fn render_lens(
        &self,
        view: View,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        if self.drawings_showing().is_empty() && self.lens.pins.is_empty() {
            return Vec::new();
        }
        let screen = |rect: Rect| {
            let at = view.point((rect.x, rect.y));
            Rect {
                x: f32::from(at.x),
                y: f32::from(at.y),
                width: rect.width * view.camera.zoom,
                height: rect.height * view.camera.zoom,
            }
        };
        let picture = self.lens_picture(&screen);
        let agent = agent_accent(cx);
        let theme = cx.theme();
        let colour = |pinned: bool| if pinned { agent.opacity(0.55) } else { agent };
        let size = view.size;
        let canvas_rect = Rect {
            x: 0.,
            y: 0.,
            width: size.0,
            height: size.1,
        };
        let lines: Vec<Line> = picture
            .paths
            .iter()
            .map(|path| Line {
                from: point(px(path.from.0), px(path.from.1)),
                to: point(px(path.to.0), px(path.to.1)),
                colour: colour(path.pinned),
                width: path.width,
                dashed: path.dashed,
                across: false,
                zoom: 1.4,
                pulse: None,
                heads: (false, true),
            })
            .collect();
        let paint = canvas(
            |_, _, _| {},
            move |drawn, _, window, _| {
                for line in lines {
                    line.paint(drawn.origin, window);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element();
        let mut out = vec![paint];
        for group in &picture.groups {
            let colour = colour(group.pinned);
            out.push(
                div()
                    .absolute()
                    .left(px(group.rect.x))
                    .top(px(group.rect.y))
                    .w(px(group.rect.width))
                    .h(px(group.rect.height))
                    .rounded(px(16.))
                    .border_2()
                    .border_dashed()
                    .border_color(colour)
                    .bg(colour.opacity(0.04))
                    .child(
                        div()
                            .absolute()
                            .left(px(10.))
                            .top(px(-11.))
                            .px_1p5()
                            .rounded(px(5.))
                            .bg(theme.background)
                            .text_xs()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colour)
                            .child(group.label.clone()),
                    )
                    .into_any_element(),
            );
        }
        let mut words = Vec::new();
        let mut current = Vec::new();
        let mut edges = Vec::new();
        for marked in &picture.boxes {
            let rect = marked.rect;
            let colour = colour(marked.pinned);
            // Touring, the step's box stands out and the rest fall back.
            let faded = picture.touring && !marked.toured;
            let fade = if faded { 0.35 } else { 1. };
            if !rect.overlaps(&canvas_rect) {
                edges.push(self.render_edge_arrow(&marked.key, marked.drawn, rect, size, cx));
                continue;
            }
            out.push(
                div()
                    .absolute()
                    .left(px(rect.x - 4.))
                    .top(px(rect.y - 4.))
                    .w(px(rect.width + 8.))
                    .h(px(rect.height + 8.))
                    .rounded(px(10.))
                    .border_2()
                    .border_color(colour)
                    .bg(colour.opacity(if marked.toured { 0.2 } else { 0.07 }))
                    .when(marked.toured, |this| this.border_4())
                    .opacity(fade)
                    .into_any_element(),
            );
            for (n, badge) in marked.badges.iter().enumerate() {
                out.push(
                    div()
                        .absolute()
                        .left(px(rect.right()
                            - 26.
                            - (marked.badges.len() - 1 - n) as f32 * 44.))
                        .top(px(rect.y - 14.))
                        .h(px(22.))
                        .min_w(px(22.))
                        .px_1p5()
                        .rounded_full()
                        .bg(colour)
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_xs()
                        .font_weight(FontWeight::BOLD)
                        .text_color(white())
                        .child(badge.clone())
                        .opacity(fade)
                        .into_any_element(),
                );
            }
            if !marked.words.is_empty() {
                let active = marked.toured;
                let explanation = v_flex()
                    .absolute()
                    .left(px(rect.x))
                    .when(marked.above, |this| this.bottom(px(size.1 - rect.y + 22.)))
                    .when(!marked.above, |this| this.top(px(rect.bottom() + 8.)))
                    .max_w(px(rect.width.max(if active { 320. } else { 260. })))
                    .px_2p5()
                    .py_1p5()
                    .gap_1()
                    .rounded(px(8.))
                    .bg(theme.background)
                    .border_1()
                    .border_color(colour.opacity(0.7))
                    .shadow_md()
                    .when(marked.proposed, |this| this.border_dashed())
                    .text_sm()
                    .text_color(theme.foreground)
                    .when(active, |this| {
                        this.border_2()
                            .border_color(colour)
                            .shadow_lg()
                            .text_base()
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .opacity(fade)
                    .children(
                        capped(&marked.words)
                            .into_iter()
                            .map(|line| div().child(line)),
                    )
                    .into_any_element();
                if active {
                    current.push(explanation);
                } else {
                    words.push(explanation);
                }
            }
        }
        for ghost in &picture.ghosts {
            out.push(
                v_flex()
                    .absolute()
                    .left(px(ghost.rect.x))
                    .top(px(ghost.rect.y))
                    .w(px(ghost.rect.width))
                    .min_h(px(ghost.rect.height))
                    .p_1p5()
                    .gap_0p5()
                    .rounded(px(8.))
                    .border_2()
                    .border_dashed()
                    .border_color(agent)
                    .bg(theme.background.opacity(0.9))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(agent)
                            .truncate()
                            .child(ghost.name.clone()),
                    )
                    .when(!ghost.purpose.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme.foreground)
                                .child(ghost.purpose.clone()),
                        )
                    })
                    .into_any_element(),
            );
            if let Some(id) = ghost.proposal {
                let name = ghost.name.clone();
                out.push(
                    div()
                        .id(SharedString::from(format!("map-adopt-{id}-{name}")))
                        .debug_selector({
                            let name = name.clone();
                            move || format!("map-adopt-{name}")
                        })
                        .absolute()
                        .left(px(ghost.rect.right() - 52.))
                        .top(px(ghost.rect.y + 4.))
                        .h(px(18.))
                        .px_1p5()
                        .rounded(px(4.))
                        .bg(theme.background)
                        .border_1()
                        .border_color(agent.opacity(0.6))
                        .text_xs()
                        .text_color(agent)
                        .hover(|style| style.bg(agent.opacity(0.12)))
                        .child("Adopt")
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(
                                "Make this box part of your sketch",
                            )
                            .build(window, cx)
                        })
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |view, _, _, cx| {
                            view.adopt(id, Some(&name), cx);
                        }))
                        .into_any_element(),
                );
            }
        }
        for tag in &picture.tags {
            out.push(
                div()
                    .absolute()
                    .left(px(tag.at.0 - 60.))
                    .top(px(tag.at.1 - 11.))
                    .max_w(px(220.))
                    .px_1p5()
                    .py_0p5()
                    .rounded(px(5.))
                    .bg(theme.background.opacity(0.95))
                    .border_1()
                    .border_dashed()
                    .border_color(agent)
                    .text_xs()
                    .text_color(theme.foreground)
                    .child(tag.text.clone())
                    .into_any_element(),
            );
        }
        for (id, first) in &picture.proposals {
            out.push(self.render_proposal_buttons(*id, *first, cx));
        }
        out.extend(words);
        out.extend(current);
        out.extend(edges);
        out.extend(self.render_legend(cx));
        out.extend(self.render_tour_bar(cx));
        out
    }

    /// An arrow at the canvas edge towards box `key`, out of view: a click
    /// flies there.
    pub(super) fn render_edge_arrow(
        &self,
        key: &str,
        drawn: Rect,
        on_screen: Rect,
        (width, height): (f32, f32),
        cx: &Context<Self>,
    ) -> AnyElement {
        let agent = agent_accent(cx);
        let (x, y, arrow) = edge_spot(on_screen, (width, height));
        div()
            .id(SharedString::from(format!("map-edge-{key}")))
            .debug_selector(|| "map-edge-arrow".into())
            .absolute()
            .left(px(x - 13.))
            .top(px(y - 13.))
            .size(px(26.))
            .rounded_full()
            .bg(agent)
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .font_weight(FontWeight::BOLD)
            .text_color(white())
            .shadow_md()
            .child(arrow)
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |view, _, _, cx| view.pan_to(drawn, cx)))
            .into_any_element()
    }
}
