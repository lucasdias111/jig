//! Drawing a sketch's boxes, arrows, notes and frames.

use super::*;

pub(super) fn render_note(note: &DrawnNote, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let (paper, words) = note_colours(theme.mode.is_dark());
    let r = note.rect;
    let font = (r.height * 0.17).min(15.);
    let key = note.key.clone();
    div()
        .debug_selector(move || key.clone())
        .absolute()
        .left(px(r.x))
        .top(px(r.y))
        .w(px(r.width))
        .min_h(px(r.height))
        .p(px((r.height * 0.1).min(8.)))
        .rounded(px(3.))
        .bg(paper)
        .shadow_md()
        .when(note.selected, |this| {
            this.border_2().border_color(theme.primary)
        })
        .when(font >= SMALLEST_TEXT, |this| {
            this.text_size(px(font))
                .text_color(words)
                .child(note.text.clone())
        })
        .into_any_element()
}

/// A sticky note's paper and words.
pub(in crate::ui::view) fn note_colours(dark: bool) -> (Hsla, Hsla) {
    if dark {
        (
            hsla(46. / 360., 0.45, 0.32, 1.),
            hsla(46. / 360., 0.3, 0.92, 1.),
        )
    } else {
        (
            hsla(48. / 360., 0.95, 0.8, 1.),
            hsla(40. / 360., 0.3, 0.18, 1.),
        )
    }
}

impl MapView {
    /// The sketches over the scene seen through `view`, the marks of other
    /// levels' sketches, a shape being drawn and a label being edited.
    pub(in crate::ui::view) fn render_sketches(
        &self,
        view: View,
        cx: &Context<Self>,
    ) -> Vec<AnyElement> {
        let screen = |rect: Rect| {
            let at = view.point((rect.x, rect.y));
            Rect {
                x: f32::from(at.x),
                y: f32::from(at.y),
                width: rect.width * view.camera.zoom,
                height: rect.height * view.camera.zoom,
            }
        };
        let picture = self.sketch_picture(&screen);
        let theme = cx.theme();
        let ink = ink(cx);
        let mut out: Vec<AnyElement> = Vec::new();
        for frame in &picture.frames {
            let r = frame.rect;
            let colour = if frame.selected {
                theme.primary
            } else {
                ink.opacity(0.55)
            };
            let key = frame.key.clone();
            out.push(
                div()
                    .debug_selector(move || key.clone())
                    .absolute()
                    .left(px(r.x))
                    .top(px(r.y))
                    .w(px(r.width))
                    .h(px(r.height))
                    .rounded(px(14.))
                    .border_2()
                    .border_dashed()
                    .border_color(colour)
                    .bg(ink.opacity(0.025))
                    .child(
                        div()
                            .absolute()
                            .left(px(6.))
                            .top(px(-22.))
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(colour)
                            .whitespace_nowrap()
                            .child(frame.label.clone()),
                    )
                    .into_any_element(),
            );
        }
        let mut lines: Vec<Line> = picture
            .arrows
            .iter()
            .map(|arrow| Line {
                from: point(px(arrow.from.0), px(arrow.from.1)),
                to: point(px(arrow.to.0), px(arrow.to.1)),
                colour: if arrow.selected { theme.primary } else { ink },
                width: if arrow.selected { 2.6 } else { 1.8 },
                dashed: !arrow.real,
                across: false,
                zoom: 1.2,
                pulse: None,
                heads: (false, true),
            })
            .collect();
        lines.extend(picture.ties.iter().map(|(from, to)| Line {
            from: point(px(from.0), px(from.1)),
            to: point(px(to.0), px(to.1)),
            colour: ink.opacity(0.45),
            width: 1.,
            dashed: true,
            across: false,
            zoom: 0.6,
            pulse: None,
            heads: (false, false),
        }));
        if let Some(preview) = self.render_preview(&screen, cx) {
            match preview {
                Preview::Line(line) => lines.push(line),
                Preview::Element(element) => out.push(element),
            }
        }
        out.push(
            canvas(
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
            .into_any_element(),
        );
        for arrow in picture.arrows.iter().filter(|a| !a.label.is_empty()) {
            let middle = bow_point(
                point(px(arrow.from.0), px(arrow.from.1)),
                point(px(arrow.to.0), px(arrow.to.1)),
                0.5,
            );
            out.push(
                div()
                    .absolute()
                    .left(middle.x - px(50.))
                    .top(middle.y - px(10.))
                    .w(px(100.))
                    .flex()
                    .justify_center()
                    .child(
                        div()
                            .px_1p5()
                            .rounded(px(4.))
                            .bg(theme.background.opacity(0.92))
                            .text_xs()
                            .text_color(theme.foreground)
                            .whitespace_nowrap()
                            .child(arrow.label.clone()),
                    )
                    .into_any_element(),
            );
        }
        for b in &picture.boxes {
            out.push(self.render_sketch_box(b, cx));
        }
        for note in &picture.notes {
            out.push(render_note(note, cx));
        }
        for bar in &picture.bars {
            out.push(self.render_bar(bar, cx));
        }
        out.extend(self.render_marks(&screen, cx));
        out.extend(self.render_sketch_field(&screen, cx));
        out
    }

    pub(super) fn render_sketch_box(&self, b: &DrawnBox, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let ink = ink(cx);
        let r = b.rect;
        let font = (r.height * 0.24).min(18.);
        let small = (font * 0.72).max(8.);
        let real = matches!(b.state, BoxState::Real(_));
        let border = if b.selected { theme.primary } else { ink };
        let status = match &b.state {
            BoxState::Real(place) => Some((format!("✓ {}", short_place(place)), theme.green)),
            BoxState::Unsure(n) => Some((format!("? matches {n} things"), theme.muted_foreground)),
            BoxState::Sketched => None,
        };
        let key = b.key.clone();
        v_flex()
            .debug_selector(move || key.clone())
            .absolute()
            .left(px(r.x))
            .top(px(r.y))
            .w(px(r.width))
            .h(px(r.height))
            .px(px((r.height * 0.12).min(10.)))
            .justify_center()
            .overflow_hidden()
            .rounded(px((r.height * 0.14).clamp(3., 10.)))
            .border(px(if b.selected { 2.5 } else { 1.6 }))
            .border_color(border)
            .when(!real, |this| {
                this.border_dashed().bg(theme.background.opacity(0.7))
            })
            .when(real, |this| this.bg(theme.background).shadow_sm())
            .when(font >= SMALLEST_TEXT, |this| {
                this.child(
                    div()
                        .text_size(px(font))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.foreground)
                        .truncate()
                        .child(b.label.clone()),
                )
                .when_some(status, |this, (text, colour)| {
                    this.child(
                        div()
                            .text_size(px(small))
                            .text_color(colour)
                            .truncate()
                            .child(text),
                    )
                })
                .when(!b.purpose.is_empty() && r.height > 70., |this| {
                    this.child(
                        div()
                            .text_size(px(small))
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(b.purpose.clone()),
                    )
                })
            })
            .into_any_element()
    }
}
