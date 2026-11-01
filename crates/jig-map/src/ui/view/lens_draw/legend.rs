//! The list of drawings, naming each and its steps.

use super::*;

pub(super) fn legend_button(
    id: (&'static str, usize),
    label: &'static str,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    h_flex()
        .id(id)
        .debug_selector(move || format!("{}-{}", id.0, id.1))
        .flex_none()
        .h(px(18.))
        .px_1p5()
        .rounded(px(4.))
        .border_1()
        .border_color(theme.border)
        .text_color(theme.foreground)
        .hover(|style| style.bg(theme.foreground.opacity(0.08)))
        .child(label)
}

/// A stop's words when several steps land on its box: the first and last
/// labels.
pub(super) fn stop_words(drawing: &Drawing, stop: &Stop) -> String {
    let label = |step: usize| drawing.steps[step].label.clone();
    match (stop.steps.first(), stop.steps.last()) {
        (Some(&first), Some(&last)) => format!(
            "{}–{} {} … {}",
            first + 1,
            last + 1,
            label(first),
            label(last)
        ),
        _ => String::new(),
    }
}

impl MapView {
    /// The list of drawings in the corner: each with its steps, those this
    /// level can't show greyed; a step clicked flies there.
    pub(super) fn render_legend(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let showing = self.drawings_showing();
        let orphans = self.orphaned_pins();
        if showing.is_empty() && orphans.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let agent = agent_accent(cx);
        let tour = self.lens.tour;
        let mut rows: Vec<AnyElement> = Vec::new();
        for (n, (source, drawing)) in showing.iter().enumerate() {
            let source = *source;
            let pinned = match source {
                Source::Pin(ix) => self.lens.pins.get(ix).map(|pin| pin.name.clone()),
                Source::Conversation(_) => None,
            };
            let header = h_flex()
                .gap_1p5()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(if pinned.is_some() {
                            agent.opacity(0.75)
                        } else {
                            agent
                        })
                        .child(match &pinned {
                            Some(_) => format!("Pinned · {}", drawing.summary()),
                            None => drawing.summary(),
                        }),
                )
                .when(drawing.kind.stepped(), |this| {
                    this.child(
                        legend_button(("map-tour", n), "Step through  →", cx).on_click(
                            cx.listener(move |view, _, _, cx| view.start_tour(source, cx)),
                        ),
                    )
                })
                .when(drawing.kind == Kind::Describe && pinned.is_none(), |this| {
                    this.child(
                        legend_button(("map-keep", n), "Keep", cx).on_click(cx.listener(
                            move |view, _, _, cx| view.settle_descriptions(source, true, cx),
                        )),
                    )
                    .child(
                        legend_button(("map-reject", n), "Reject", cx).on_click(cx.listener(
                            move |view, _, _, cx| view.settle_descriptions(source, false, cx),
                        )),
                    )
                })
                .when_some(pinned, |this, name| {
                    this.child(
                        legend_button(("map-unpin", n), "Unpin", cx)
                            .on_click(cx.listener(move |view, _, _, cx| view.unpin(&name, cx))),
                    )
                });
            let steps = drawing.steps.iter().enumerate().map(|(ix, step)| {
                let landed = self.landing(&step.place).is_some();
                let toured = tour.is_some_and(|t| t.source == source && t.step == ix);
                let place = step.place.clone();
                let text = match drawing.kind {
                    Kind::Trace | Kind::Tour => {
                        format!(
                            "{}. {} — {}",
                            ix + 1,
                            step.label,
                            self.short_name(&step.place)
                        )
                    }
                    Kind::Describe => format!("{}: {}", self.short_name(&step.place), step.label),
                    _ => self.short_name(&step.place),
                };
                div()
                    .id(("map-legend-step", n * 1000 + ix))
                    .debug_selector(move || format!("map-legend-step-{n}-{ix}"))
                    .cursor_pointer()
                    .text_color(if landed {
                        theme.foreground
                    } else {
                        theme.muted_foreground.opacity(0.6)
                    })
                    .when(toured, |this| {
                        this.text_color(agent).font_weight(FontWeight::SEMIBOLD)
                    })
                    .hover(|style| style.text_color(agent))
                    .child(text)
                    .on_click(cx.listener(move |view, _, _, cx| {
                        view.fly_to(&place, cx);
                    }))
            });
            rows.push(
                v_flex()
                    .gap_0p5()
                    .child(header)
                    .when(drawing.kind != Kind::Note, |this| this.children(steps))
                    .into_any_element(),
            );
        }
        for (name, gone) in orphans {
            rows.push(
                div()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "Orphaned pin {name}: {gone} place{} gone",
                        if gone == 1 { "" } else { "s" }
                    ))
                    .into_any_element(),
            );
        }
        let right = self.list_goes_right();
        Some(
            v_flex()
                .id("map-lens-list")
                .debug_selector(|| "map-lens-list".into())
                .absolute()
                .when(right, |this| this.top(px(super::super::ruler::RULER_ROOM)))
                .when(!right, |this| this.top(px(LIST_TOP)))
                .when(right, |this| this.right(px(12.)))
                .when(!right, |this| this.left(px(12.)))
                .w(px(LIST_WIDTH))
                .max_h(relative(0.7))
                .overflow_y_scroll()
                .p_2p5()
                .gap_2()
                .rounded(px(8.))
                .bg(theme.background.opacity(0.95))
                .border_1()
                .border_color(agent.opacity(0.45))
                .shadow_md()
                .text_xs()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .children(rows)
                .into_any_element(),
        )
    }

    /// Whether the list of drawings goes top right: when the conversation
    /// covers the top left of the canvas, where it goes otherwise.
    pub fn list_goes_right(&self) -> bool {
        let Some(chat) = self.lens.conversation.as_ref().map(|bounds| bounds.get()) else {
            return false;
        };
        if chat.size.width <= px(0.) {
            return false;
        }
        let canvas = self.bounds.get();
        let list = Bounds {
            origin: canvas.origin + point(px(12.), px(LIST_TOP)),
            size: size(px(LIST_WIDTH), canvas.size.height * 0.7),
        };
        list.intersects(&chat)
    }

    /// A place as the list names it: a function's title, a file's name, a
    /// folder's last part, a domain's name.
    pub(in crate::ui::view) fn short_name(&self, name: &str) -> String {
        match self.place(name) {
            Some(Place::File {
                function: Some(function),
                ..
            }) => function,
            Some(Place::File {
                path,
                owner: Some(owner),
                ..
            }) => format!("{owner} in {}", file_name(&path)),
            Some(Place::File { path, .. }) => file_name(&path),
            Some(Place::Folder(dir)) => format!("{}/", file_name(dir.trim_end_matches('/'))),
            Some(Place::Domain(name)) => name,
            Some(Place::Sketch { sketch, shape }) => self.sketch_box_label(&sketch, &shape),
            None => name.to_string(),
        }
    }
}
