//! Drawing the user's sketches, in neutral ink apart from the agent's
//! orange: boxes dashed while they're ideas, solid once they're real code
//! (a "?" when the name could be several things), arrows dashed until the
//! code calls along them, yellow sticky notes, dashed frames. Above each
//! sketch a bar with its title, how much of it is real, and what to do
//! with it: Make Task, its task and Implement, Archive. Elsewhere, a mark
//! on the boxes carrying sketches of other levels. The tool strip sits in
//! the canvas's bottom left; the Sketches list opens from the header.

use gpui_kit::component::input::Input;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::super::layout::Rect;
use super::super::sketch::{Match, ShapeKind, ShapeRef};
use super::draw::{Line, View, bow_point};
use super::sketch::{EditTarget, SketchDrag, TaskInfo, Tool};
use super::{MapView, input::Drag};

/// A box's label, from its height on screen: no smaller than this reads.
const SMALLEST_TEXT: f32 = 7.;

/// How a box of a sketch is drawn.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum BoxState {
    /// An idea: no code yet.
    Sketched,
    /// Real code, at this place.
    Real(String),
    /// Its name could be several things.
    Unsure(usize),
}

pub(super) struct DrawnBox {
    pub(super) key: String,
    pub(super) rect: Rect,
    pub(super) label: String,
    pub(super) purpose: String,
    pub(super) state: BoxState,
    pub(super) selected: bool,
}

pub(super) struct DrawnNote {
    pub(super) key: String,
    pub(super) rect: Rect,
    pub(super) text: String,
    pub(super) selected: bool,
}

pub(super) struct DrawnFrame {
    pub(super) key: String,
    pub(super) rect: Rect,
    pub(super) label: String,
    pub(super) selected: bool,
}

pub(super) struct DrawnArrow {
    pub(super) from: (f32, f32),
    pub(super) to: (f32, f32),
    pub(super) label: String,
    /// The code calls along it.
    pub(super) real: bool,
    pub(super) selected: bool,
}

/// A sketch's bar, above it.
pub(super) struct Bar {
    pub(super) name: String,
    pub(super) title: String,
    /// Its bottom left, on screen.
    pub(super) at: (f32, f32),
    pub(super) real: (usize, usize),
    pub(super) task: Option<(String, Option<TaskInfo>)>,
    pub(super) done: bool,
}

/// Every sketch at the level showing, on screen: what the canvas draws,
/// and Export Image.
#[derive(Default)]
pub(super) struct SketchPicture {
    pub(super) frames: Vec<DrawnFrame>,
    pub(super) arrows: Vec<DrawnArrow>,
    /// Notes to what they're on.
    pub(super) ties: Vec<((f32, f32), (f32, f32))>,
    pub(super) boxes: Vec<DrawnBox>,
    pub(super) notes: Vec<DrawnNote>,
    pub(super) bars: Vec<Bar>,
}

impl SketchPicture {
    pub(super) fn is_empty(&self) -> bool {
        self.bars.is_empty()
    }
}

impl MapView {
    /// The sketches at the level showing, `screen` putting a rect of the
    /// drawing on screen.
    pub(super) fn sketch_picture(&self, screen: &dyn Fn(Rect) -> Rect) -> SketchPicture {
        let mut picture = SketchPicture::default();
        for sketch in self.sketches_here() {
            let laid = self.lay_out(sketch);
            let key = |kind: ShapeKind, id: String| {
                ShapeRef {
                    sketch: sketch.name.clone(),
                    kind,
                    id,
                }
                .key()
            };
            let selected = |key: &str| self.selected.iter().any(|k| k == key);
            for (ix, rect) in &laid.frames {
                let frame = &sketch.frames[*ix];
                let key = key(ShapeKind::Frame, frame.id.clone());
                picture.frames.push(DrawnFrame {
                    selected: selected(&key),
                    key,
                    rect: screen(*rect),
                    label: frame.label.clone(),
                });
            }
            for (ix, from, to, real) in &laid.arrows {
                let arrow = &sketch.arrows[*ix];
                let (from, to) = super::scene::edge_points(screen(*from), screen(*to));
                let key = key(ShapeKind::Arrow, ix.to_string());
                picture.arrows.push(DrawnArrow {
                    from,
                    to,
                    label: arrow.label.clone(),
                    real: *real,
                    selected: selected(&key),
                });
            }
            for (ix, on) in &laid.ties {
                if let Some((_, note)) = laid.notes.iter().find(|(at, _)| at == ix) {
                    let (a, b) = (screen(*note), screen(*on));
                    if !a.overlaps(&b) {
                        picture.ties.push(super::scene::edge_points(a, b));
                    }
                }
            }
            for (ix, rect) in &laid.boxes {
                let b = &sketch.boxes[*ix];
                let key = key(ShapeKind::Box, b.id.clone());
                let state = match self.box_match(&sketch.name, &b.id) {
                    Match::Real(place) => BoxState::Real(place),
                    Match::Unsure(n) => BoxState::Unsure(n),
                    Match::None => BoxState::Sketched,
                };
                picture.boxes.push(DrawnBox {
                    selected: selected(&key),
                    key,
                    rect: screen(*rect),
                    label: b.label.clone(),
                    purpose: b.purpose.clone(),
                    state,
                });
            }
            for (ix, rect) in &laid.notes {
                let note = &sketch.notes[*ix];
                let key = key(ShapeKind::Note, note.id.clone());
                picture.notes.push(DrawnNote {
                    selected: selected(&key),
                    key,
                    rect: screen(*rect),
                    text: note.text.clone(),
                });
            }
            if let Some(bounds) = laid.bounds().map(screen) {
                // Clear of a frame's label at the top.
                let lift = if laid
                    .frames
                    .iter()
                    .any(|(_, r)| screen(*r).y <= bounds.y + 1.)
                {
                    26.
                } else {
                    8.
                };
                picture.bars.push(Bar {
                    name: sketch.name.clone(),
                    title: sketch.title.clone(),
                    at: (bounds.x, bounds.y - lift),
                    real: self.real_boxes(&sketch.name),
                    task: sketch
                        .task
                        .clone()
                        .map(|id| (id.clone(), self.sketches.tasks.get(&id).cloned())),
                    done: self.sketch_done(&sketch.name),
                });
            }
        }
        picture
    }

    /// The sketches over the scene seen through `view`, the marks of other
    /// levels' sketches, a shape being drawn and a label being edited.
    pub(super) fn render_sketches(&self, view: View, cx: &Context<Self>) -> Vec<AnyElement> {
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

    fn render_sketch_box(&self, b: &DrawnBox, cx: &Context<Self>) -> AnyElement {
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

    /// A sketch's bar: its title (double-click to rename), how much is
    /// real, and what to do with it next.
    fn render_bar(&self, bar: &Bar, cx: &Context<Self>) -> AnyElement {
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
                row = row.child(button("map-sketch-task", label, false).on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(super::MapEvent::OpenTask(id.clone()))),
                ));
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
    fn render_marks(&self, screen: &dyn Fn(Rect) -> Rect, cx: &Context<Self>) -> Vec<AnyElement> {
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
    fn render_preview(&self, screen: &dyn Fn(Rect) -> Rect, cx: &Context<Self>) -> Option<Preview> {
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

    /// The label being edited, over its shape.
    fn render_sketch_field(
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
    pub(super) fn render_tool_strip(&self, cx: &Context<Self>) -> Option<AnyElement> {
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
    pub(super) fn render_sketch_list(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.sketches.list_open {
            return None;
        }
        let theme = cx.theme();
        let ink = ink(cx);
        let mut sketches: Vec<&super::super::sketch::Sketch> =
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
                .top(px(super::ruler::RULER_ROOM))
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

enum Preview {
    Line(Line),
    Element(AnyElement),
}

fn render_note(note: &DrawnNote, cx: &App) -> AnyElement {
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
pub(super) fn note_colours(dark: bool) -> (Hsla, Hsla) {
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

/// The sketches' neutral ink.
pub(super) fn ink(cx: &App) -> Hsla {
    cx.theme().foreground.opacity(0.78)
}

/// A place shortly: a function's title, a file's name, a folder's last
/// part.
pub(super) fn short_place(place: &str) -> String {
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
