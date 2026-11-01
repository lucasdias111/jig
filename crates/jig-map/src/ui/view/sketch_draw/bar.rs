//! The bar above each sketch, the marks on boxes carrying one, and the shape being drawn.

use super::*;

impl MapView {
    /// A sketch's bar: its title (double-click to rename), how much is
    /// real, and what to do with it next.
    pub(super) fn render_bar(&self, bar: &Bar, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let ink = ink(cx);
        let name = bar.name.clone();
        let button = |id: &str, label: String, primary: bool| {
            h_flex()
                .id(SharedString::from(format!("{id}-{}", bar.name)))
                .debug_selector({
                    let id = id.to_string();
                    move || id.clone()
                })
                .flex_none()
                .h(px(20.))
                .px_1p5()
                .rounded(px(4.))
                .text_xs()
                .when(primary, |this| this.bg(ink).text_color(theme.background))
                .when(!primary, |this| {
                    this.border_1()
                        .border_color(theme.border)
                        .bg(theme.background)
                        .text_color(theme.foreground)
                        .hover(|style| style.bg(theme.foreground.opacity(0.08)))
                })
                .child(label)
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        };
        let (real, total) = bar.real;
        let mut row = h_flex()
            .absolute()
            .left(px(bar.at.0))
            .top(px(bar.at.1 - 28.))
            .gap_1p5()
            .px_1p5()
            .py_0p5()
            .rounded(px(6.))
            .bg(theme.background.opacity(0.94))
            .shadow_sm()
            .text_xs()
            .child(
                div()
                    .id(SharedString::from(format!("map-sketch-title-{}", bar.name)))
                    .debug_selector(|| "map-sketch-title".into())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(ink)
                    .whitespace_nowrap()
                    .child(format!("✎ {}", bar.title))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener({
                        let name = name.clone();
                        move |view, event: &ClickEvent, window, cx| {
                            if event.click_count() >= 2 {
                                view.begin_sketch_edit(
                                    EditTarget::Title(name.clone()),
                                    false,
                                    window,
                                    cx,
                                );
                            } else {
                                view.select_sketch(&name);
                                view.sketches.active = Some(name.clone());
                                cx.notify();
                            }
                        }
                    }))
                    .tooltip(|window, cx| {
                        Tooltip::new(
                            "Click to select it all, then ⌘K to ask · double-click to rename",
                        )
                        .build(window, cx)
                    }),
            )
            .when(total > 0, |this| {
                this.child(
                    div()
                        .text_color(if real == total {
                            theme.green
                        } else {
                            theme.muted_foreground
                        })
                        .whitespace_nowrap()
                        .child(format!("{real} of {total} real")),
                )
            });
        match &bar.task {
            None => {
                row = row.child(
                    button("map-sketch-make-task", "Make Task".into(), false).on_click(
                        cx.listener({
                            let name = name.clone();
                            move |view, _, _, cx| view.make_task(&name, cx)
                        }),
                    ),
                );
            }
            Some((id, info)) => {
                let label = match info {
                    Some(info) => format!("Task: {} · {}/{}", info.title, info.done, info.total),
                    None => format!("Task: {id} (gone)"),
                };
                let id = id.clone();
                row = row.child(
                    button("map-sketch-task", label, false).on_click(cx.listener(
                        move |_, _, _, cx| cx.emit(super::super::MapEvent::OpenTask(id.clone())),
                    )),
                );
                if info.as_ref().is_some_and(|info| !info.finished()) {
                    row = row.child(
                        button("map-sketch-implement", "Implement".into(), false).on_click(
                            cx.listener({
                                let name = name.clone();
                                move |view, _, _, cx| view.implement_sketch(&name, cx)
                            }),
                        ),
                    );
                }
            }
        }
        if bar.done {
            row = row.child(
                button("map-sketch-archive", "Archive".into(), true).on_click(
                    cx.listener(move |view, _, _, cx| view.set_archived(&name, true, cx)),
                ),
            );
        }
        row.into_any_element()
    }

    /// Marks on the boxes carrying sketches drawn at other levels: a
    /// click goes there.
    pub(super) fn render_marks(
        &self,
        screen: &dyn Fn(Rect) -> Rect,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = cx.theme();
        let ink = ink(cx);
        self.sketch_marks()
            .into_iter()
            .enumerate()
            .map(|(n, (landing, names))| {
                let rect = screen(landing.rect);
                let titles: Vec<String> = names
                    .iter()
                    .filter_map(|name| self.sketch(name).map(|s| s.title.clone()))
                    .collect();
                let label = match titles.as_slice() {
                    [one] => format!("✎ {one}"),
                    many => format!("✎ {} sketches", many.len()),
                };
                let tip = format!("Go to {}", titles.join(", "));
                let first = names[0].clone();
                div()
                    .id(("map-sketch-mark", n))
                    .debug_selector(|| "map-sketch-mark".into())
                    .absolute()
                    .left(px(rect.right() - 8.))
                    .top(px(rect.y - 10.))
                    .h(px(20.))
                    .px_1p5()
                    .flex()
                    .items_center()
                    .rounded_full()
                    .border_1()
                    .border_dashed()
                    .border_color(ink)
                    .bg(theme.background)
                    .text_xs()
                    .text_color(ink)
                    .whitespace_nowrap()
                    .shadow_sm()
                    .child(label)
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |view, _, _, cx| view.go_to_sketch(&first, cx)))
                    .into_any_element()
            })
            .collect()
    }

    /// The shape a tool is dragging out.
    pub(super) fn render_preview(
        &self,
        screen: &dyn Fn(Rect) -> Rect,
        cx: &Context<Self>,
    ) -> Option<Preview> {
        let Some(Drag::Sketch(SketchDrag::Draw {
            tool,
            from,
            to,
            start,
        })) = &self.drag
        else {
            return None;
        };
        let origin = self.bounds.get().origin;
        let (from, to) = (*from - origin, *to - origin);
        let ink = ink(cx);
        if *tool == Tool::Arrow {
            start.as_ref()?;
            return Some(Preview::Line(Line {
                from,
                to,
                colour: ink,
                width: 1.8,
                dashed: true,
                across: false,
                zoom: 1.2,
                pulse: None,
                heads: (false, true),
            }));
        }
        let _ = screen;
        let (x, y) = (from.x.min(to.x), from.y.min(to.y));
        let (w, h) = ((from.x - to.x).abs(), (from.y - to.y).abs());
        Some(Preview::Element(
            div()
                .absolute()
                .left(x)
                .top(y)
                .w(w)
                .h(h)
                .rounded(px(8.))
                .border_1()
                .border_dashed()
                .border_color(ink)
                .into_any_element(),
        ))
    }
}
