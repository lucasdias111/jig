//! The tool strip, the label field, and the header's list of sketches.

use super::*;

/// A place shortly: a function's title, a file's name, a folder's last
/// part.
pub(in crate::ui::view) fn short_place(place: &str) -> String {
    if let Some((_, inner)) = place.split_once('#') {
        return inner.to_string();
    }
    let trimmed = place.trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or(trimmed);
    if place.ends_with('/') {
        format!("{last}/")
    } else {
        last.to_string()
    }
}

impl MapView {
    /// The label being edited, over its shape.
    pub(super) fn render_sketch_field(
        &self,
        screen: &dyn Fn(Rect) -> Rect,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        let editing = self.sketches.editing.as_ref()?;
        let rect = match &editing.target {
            EditTarget::Shape(shape) => screen(self.shape_rect(shape)?),
            EditTarget::Title(name) => {
                let sketch = self.sketch(name)?;
                let bounds = screen(self.lay_out(sketch).bounds()?);
                Rect {
                    y: bounds.y - 34.,
                    height: 0.,
                    ..bounds
                }
            }
        };
        let width = rect.width.clamp(200., 420.);
        let (left, top) = match &editing.target {
            EditTarget::Shape(ShapeRef {
                kind: ShapeKind::Arrow,
                ..
            }) => {
                let (x, y) = rect.centre();
                (x - width / 2., y - 14.)
            }
            EditTarget::Shape(ShapeRef {
                kind: ShapeKind::Frame,
                ..
            }) => (rect.x, rect.y - 30.),
            EditTarget::Shape(_) => (rect.x, rect.centre().1 - 14.),
            EditTarget::Title(_) => (rect.x, rect.y),
        };
        let theme = cx.theme();
        Some(
            div()
                .id("map-sketch-field")
                .absolute()
                .left(px(left))
                .top(px(top))
                .w(px(width))
                .rounded(px(6.))
                .bg(theme.background)
                .shadow_md()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(Input::new(&editing.input).small().cleanable(false))
                .into_any_element(),
        )
    }

    /// The tools, in the canvas's bottom left.
    pub(in crate::ui::view) fn render_tool_strip(&self, cx: &Context<Self>) -> Option<AnyElement> {
        self.project.as_ref()?;
        let theme = cx.theme();
        let picked = self.sketches.tool;
        let glyph = |tool: Tool| match tool {
            Tool::Pointer => "↖",
            Tool::Box => "▭",
            Tool::Arrow => "→",
            Tool::Note => "✎",
            Tool::Frame => "⬚",
        };
        Some(IntoElement::into_any_element(
            v_flex()
                .id("map-tools")
                .debug_selector(|| "map-tools".into())
                .absolute()
                .left(px(12.))
                .bottom(px(32.))
                .p(px(3.))
                .gap(px(2.))
                .rounded(px(8.))
                .bg(theme.background.opacity(0.95))
                .border_1()
                .border_color(theme.border)
                .shadow_md()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .children(Tool::ALL.into_iter().map(|tool| {
                    let tip = match tool {
                        Tool::Pointer => "Pointer (Esc)".to_string(),
                        tool => format!("{} ({})", tool.name(), tool.key().to_uppercase()),
                    };
                    let on = tool == picked;
                    div()
                        .id(SharedString::from(format!("map-tool-{}", tool.name())))
                        .debug_selector(move || format!("map-tool-{}", tool.name()))
                        .size(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(5.))
                        .text_base()
                        .when(on, |this| {
                            this.bg(theme.foreground.opacity(0.12))
                                .text_color(theme.foreground)
                        })
                        .when(!on, |this| {
                            this.text_color(theme.muted_foreground)
                                .hover(|style| style.bg(theme.foreground.opacity(0.06)))
                        })
                        .child(glyph(tool))
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                        .on_click(cx.listener(move |view, _, _, cx| view.set_tool(tool, cx)))
                })),
        ))
    }

    /// The Sketches list: every sketch, where it is and how far it got;
    /// archived ones at the end, to restore.
    pub(in crate::ui::view) fn render_sketch_list(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.sketches.list_open {
            return None;
        }
        let theme = cx.theme();
        let ink = ink(cx);
        let mut sketches: Vec<&crate::domain::sketch::Sketch> =
            self.sketches.sketches.iter().collect();
        sketches.sort_by_key(|s| s.archived);
        let rows = sketches.into_iter().enumerate().map(|(n, sketch)| {
            let name = sketch.name.clone();
            let (real, total) = self.real_boxes(&sketch.name);
            let level = match sketch.level.as_str() {
                "site" => "Site".to_string(),
                "domain" => "Domain".to_string(),
                level => format!("Plan of {}", short_place(level.trim_start_matches("plan:"))),
            };
            let task = sketch
                .task
                .as_ref()
                .and_then(|id| self.sketches.tasks.get(id))
                .map(|t| format!(" · task {}/{}", t.done, t.total))
                .unwrap_or_default();
            let small = |id: &'static str, label: &'static str| {
                div()
                    .id((id, n))
                    .debug_selector(move || format!("{id}-{n}"))
                    .px_1()
                    .rounded(px(3.))
                    .text_color(theme.muted_foreground)
                    .hover(|style| style.text_color(theme.foreground))
                    .child(label)
            };
            h_flex()
                .gap_2()
                .child(
                    v_flex()
                        .id(("map-sketch-row", n))
                        .debug_selector(move || format!("map-sketch-row-{n}"))
                        .flex_1()
                        .min_w_0()
                        .cursor_pointer()
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(if sketch.archived {
                                    theme.muted_foreground
                                } else {
                                    ink
                                })
                                .truncate()
                                .child(format!(
                                    "{}{}",
                                    sketch.title,
                                    if sketch.archived { " (archived)" } else { "" }
                                )),
                        )
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .truncate()
                                .child(format!("{level} · {real} of {total} real{task}")),
                        )
                        .when(!sketch.archived, |this| {
                            let name = name.clone();
                            this.on_click(
                                cx.listener(move |view, _, _, cx| view.go_to_sketch(&name, cx)),
                            )
                        }),
                )
                .when(sketch.archived, |this| {
                    let name = name.clone();
                    this.child(small("map-sketch-restore", "Restore").on_click(
                        cx.listener(move |view, _, _, cx| view.set_archived(&name, false, cx)),
                    ))
                })
                .child(
                    small("map-sketch-delete", "Delete")
                        .on_click(cx.listener(move |view, _, _, cx| view.delete_sketch(&name, cx))),
                )
        });
        Some(
            v_flex()
                .id("map-sketch-list")
                .debug_selector(|| "map-sketch-list".into())
                .absolute()
                .top(px(super::super::ruler::RULER_ROOM))
                .right(px(12.))
                .w(px(320.))
                .max_h(relative(0.7))
                .overflow_y_scroll()
                .p_2p5()
                .gap_2()
                .rounded(px(8.))
                .bg(theme.background.opacity(0.97))
                .border_1()
                .border_color(theme.border)
                .shadow_md()
                .text_xs()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .child("Sketches"),
                )
                .when(self.sketches.sketches.is_empty(), |this| {
                    this.child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child("None yet: pick the box tool (B) and draw."),
                    )
                })
                .children(rows)
                .into_any_element(),
        )
    }
}
