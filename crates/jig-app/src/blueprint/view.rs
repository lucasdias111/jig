//! The blueprint view: the file's functions down the left, and the one
//! picked drawn as blocks joined by exec wires, which drag, scroll and
//! pinch to move around. Thinner data wires, coloured by name, join each
//! binding to the blocks reading it; the header's switch hides them.
//! Clicking a block asks for its code; the functions calling this one
//! sit to the left of its start, and a call's button opens what it calls.

use std::cell::Cell;
use std::collections::HashSet;
use std::ops::Range;
use std::rc::Rc;

use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::debug::{self, DebugMarks, OnBlocks};
use super::layout::{self, Layout, Rect};
use super::{Caller, Function, Graph, Kind, Node};

const LIST_WIDTH: f32 = 210.;
const MIN_ZOOM: f32 = 0.25;
const MAX_ZOOM: f32 = 2.;
/// Where the drawing starts, from the canvas's top-left corner.
const MARGIN: f32 = 40.;
const GRID: f32 = 24.;
const PIN_SIZE: f32 = 9.;
const DATA_PIN_SIZE: f32 = 7.;
const HEADER_HEIGHT: f32 = 34.;
/// The blocks for callers, left of the start block.
const CALLER_WIDTH: f32 = 190.;
const CALLER_GAP: f32 = 70.;
const CALLER_HEIGHT: f32 = layout::HEADER_HEIGHT + layout::LINE_HEIGHT + 2. * layout::BODY_PADDING;
const CALLER_SPACING: f32 = 12.;
/// Room above the drawing for the callers' label.
const LABEL_HEIGHT: f32 = 22.;

/// Where the drawing's origin starts in the canvas: room on the left for
/// callers, so the drawing doesn't move when they arrive.
fn start_offset() -> Point<Pixels> {
    point(
        px(MARGIN + CALLER_WIDTH + CALLER_GAP),
        px(MARGIN + LABEL_HEIGHT),
    )
}

pub enum BlueprintEvent {
    /// A block was clicked: show its code.
    Open(Range<usize>),
    /// Escape: back to the code as it was.
    Close,
    /// A function is drawn, from its start: find its callers.
    Shown(Function),
    /// A caller was clicked: show its blueprint at the call.
    OpenCaller(Caller),
    /// A call's button: show the blueprint of the function it calls,
    /// whose name starts at this offset.
    StepInto(usize),
    /// A block's dot: a breakpoint on the line at this offset, or none.
    ToggleBreakpoint(usize),
}

pub struct BlueprintView {
    focus: FocusHandle,
    /// The text the functions were read from.
    source: String,
    functions: Vec<Function>,
    current: Option<usize>,
    graph: Graph,
    layout: Layout,
    /// Where the drawing's origin sits in the canvas.
    offset: Point<Pixels>,
    zoom: f32,
    /// The pointer's last position while the canvas is dragged.
    panning: Option<Point<Pixels>>,
    /// The canvas, as last drawn, in window coordinates.
    bounds: Rc<Cell<Bounds<Pixels>>>,
    /// Whether data wires and their pins are drawn.
    data_wires: bool,
    /// The functions calling the one shown, `None` while they're found.
    callers: Option<Vec<Caller>>,
    /// The block a jump landed on, outlined until another is shown.
    highlight: Option<usize>,
    /// The list's sections folded shut, by name; `None` is the top of the
    /// file.
    folded: HashSet<Option<String>>,
    /// Sections already met, so a test module starts folded only once.
    seen_sections: HashSet<Option<String>>,
    /// What the debugger has to show here.
    debug: DebugMarks,
}

impl EventEmitter<BlueprintEvent> for BlueprintView {}

impl Focusable for BlueprintView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BlueprintView {
    /// The flow of the function around `cursor` in `source`.
    pub fn new(source: String, cursor: usize, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            focus: cx.focus_handle(),
            source: String::new(),
            functions: Vec::new(),
            current: None,
            graph: Graph::default(),
            layout: Layout::default(),
            offset: start_offset(),
            zoom: 1.,
            panning: None,
            bounds: Default::default(),
            data_wires: true,
            callers: None,
            highlight: None,
            folded: HashSet::new(),
            seen_sections: HashSet::new(),
            debug: DebugMarks::default(),
        };
        this.refresh(source, cursor, cx);
        this
    }

    /// Read the code again, staying on the same function, where it was,
    /// while it's still there; else the one around `cursor`.
    pub fn refresh(&mut self, source: String, cursor: usize, cx: &mut Context<Self>) {
        if source == self.source && self.current.is_some() {
            return;
        }
        let name = self
            .current_function()
            .map(|function| function.name.clone());
        self.source = source;
        self.functions = super::functions(&self.source);
        for function in &self.functions {
            if self.seen_sections.insert(function.section.clone())
                && function.section.as_deref().is_some_and(is_test_section)
            {
                self.folded.insert(function.section.clone());
            }
        }
        let same = name.and_then(|name| self.functions.iter().position(|f| f.name == name));
        match same.or_else(|| super::at(&self.functions, cursor)) {
            Some(ix) => {
                let place = (self.offset, self.zoom);
                self.show(ix, cx);
                if same.is_some() {
                    (self.offset, self.zoom) = place;
                }
            }
            None => {
                self.current = None;
                self.graph = Graph::default();
                self.layout = Layout::default();
            }
        }
        cx.notify();
    }

    pub fn current_function(&self) -> Option<&Function> {
        self.current.and_then(|ix| self.functions.get(ix))
    }

    #[cfg(test)]
    pub fn functions(&self) -> &[Function] {
        &self.functions
    }

    #[cfg(test)]
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    #[cfg(test)]
    pub fn layout(&self) -> &Layout {
        &self.layout
    }

    /// Show or hide the data wires, making room for their pins or not.
    pub fn set_data_wires(&mut self, on: bool, cx: &mut Context<Self>) {
        self.data_wires = on;
        self.layout = layout::layout(&self.graph, on);
        cx.notify();
    }

    /// Draw function `ix`, from its start.
    pub fn show(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(function) = self.functions.get(ix) else {
            return;
        };
        self.graph = super::graph(&self.source, function).unwrap_or_default();
        self.layout = layout::layout(&self.graph, self.data_wires);
        let shown = function.clone();
        // Its section opens, so it shows as picked.
        self.folded.remove(&shown.section);
        self.current = Some(ix);
        self.offset = start_offset();
        self.zoom = 1.;
        self.callers = None;
        self.highlight = None;
        cx.emit(BlueprintEvent::Shown(shown));
        cx.notify();
    }

    /// Draw the function around `offset`, with the block there outlined
    /// and in view.
    pub fn show_at(&mut self, offset: usize, cx: &mut Context<Self>) {
        let Some(ix) = super::at(&self.functions, offset) else {
            return;
        };
        if self.current != Some(ix) {
            self.show(ix, cx);
        }
        self.highlight = super::node_at(&self.graph, offset).filter(|&node| node > 0);
        if let Some(rect) = self.highlight.map(|node| self.layout.rects[node]) {
            self.bring_into_view(rect);
        }
        cx.notify();
    }

    /// Move the drawing so `rect` is on screen, centred if it wasn't.
    fn bring_into_view(&mut self, rect: Rect) {
        let size = self.bounds.get().size;
        // Not drawn yet: as wide as the window is likely to be.
        let (width, height) = if size.width > px(0.) {
            (f32::from(size.width), f32::from(size.height))
        } else {
            (800., 480.)
        };
        let left = self.to_canvas((rect.x, rect.y));
        let right = self.to_canvas((rect.right(), rect.bottom()));
        if f32::from(right.x) > width - MARGIN || f32::from(left.x) < 0. {
            self.offset.x = px(width / 2. - (rect.x + rect.width / 2.) * self.zoom);
        }
        if f32::from(right.y) > height - MARGIN || f32::from(left.y) < 0. {
            self.offset.y = px(height / 2. - (rect.y + rect.height / 2.) * self.zoom);
        }
    }

    /// The callers of `function`, found; dropped if another is shown by
    /// now.
    pub fn set_callers(&mut self, function: &str, callers: Vec<Caller>, cx: &mut Context<Self>) {
        if self
            .current_function()
            .is_some_and(|shown| shown.name == function)
        {
            self.callers = Some(callers);
            cx.notify();
        }
    }

    /// Where the view is, for Go Back: the block a jump landed on, else
    /// the function's name.
    pub fn place(&self) -> Option<usize> {
        self.highlight
            .and_then(|node| self.graph.nodes.get(node))
            .map(|node| node.range.start)
            .or_else(|| self.current_function().map(|function| function.name_at))
    }

    #[cfg(test)]
    pub fn callers(&self) -> Option<&[Caller]> {
        self.callers.as_deref()
    }

    #[cfg(test)]
    pub fn highlight(&self) -> Option<usize> {
        self.highlight
    }

    /// The debugger's marks, as they now are. Pausing somewhere new shows
    /// that function, with the block it's paused in on screen.
    pub fn set_debug(&mut self, marks: DebugMarks, cx: &mut Context<Self>) {
        if marks == self.debug {
            return;
        }
        let paused = marks.paused.filter(|&at| Some(at) != self.debug.paused);
        self.debug = marks;
        if let Some(at) = paused
            && let Some(ix) = super::at(&self.functions, at)
        {
            if self.current != Some(ix) {
                self.show(ix, cx);
            }
            if let Some(node) = super::node_at(&self.graph, at) {
                self.bring_into_view(self.layout.rects[node]);
            }
        }
        cx.notify();
    }

    #[cfg(test)]
    pub fn on_blocks(&self) -> OnBlocks {
        debug::on_blocks(&self.graph, self.function_range(), &self.debug)
    }

    /// The function shown, in the file.
    fn function_range(&self) -> Range<usize> {
        self.current_function()
            .map_or(0..0, |function| function.range.clone())
    }

    /// Node `ix`'s breakpoint, as its dot does.
    pub fn toggle_breakpoint(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(node) = self.graph.nodes.get(ix) {
            cx.emit(BlueprintEvent::ToggleBreakpoint(node.range.start));
        }
    }

    /// Node `ix`'s call, as its button does.
    pub fn step_into(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(callee) = self.graph.nodes.get(ix).and_then(|node| node.callee) {
            cx.emit(BlueprintEvent::StepInto(callee));
        }
    }

    /// Where caller `ix`'s block is, in the drawing.
    fn caller_rect(ix: usize) -> Rect {
        Rect {
            x: -(CALLER_GAP + CALLER_WIDTH),
            y: ix as f32 * (CALLER_HEIGHT + CALLER_SPACING),
            width: CALLER_WIDTH,
            height: CALLER_HEIGHT,
        }
    }

    fn caller_count(&self) -> usize {
        self.callers.as_ref().map_or(0, Vec::len)
    }

    /// Click through to node `ix`'s code, as a click on it does.
    pub fn open_node(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(node) = self.graph.nodes.get(ix) {
            cx.emit(BlueprintEvent::Open(node.range.clone()));
        }
    }

    /// A point in the window as one in the drawing, unzoomed.
    fn to_drawing(&self, position: Point<Pixels>) -> (f32, f32) {
        let local = position - self.bounds.get().origin - self.offset;
        (
            f32::from(local.x) / self.zoom,
            f32::from(local.y) / self.zoom,
        )
    }

    /// A point in the drawing as one in the canvas.
    fn to_canvas(&self, (x, y): (f32, f32)) -> Point<Pixels> {
        point(
            self.offset.x + px(x * self.zoom),
            self.offset.y + px(y * self.zoom),
        )
    }

    /// Whether a block, or a caller's, is at `position`.
    fn block_at(&self, position: Point<Pixels>) -> bool {
        let (x, y) = self.to_drawing(position);
        let inside =
            |rect: &Rect| x >= rect.x && x <= rect.right() && y >= rect.y && y <= rect.bottom();
        self.layout.rects.iter().any(inside)
            || (0..self.caller_count()).any(|ix| inside(&Self::caller_rect(ix)))
    }

    /// Zoom by `factor`, keeping the drawing still under `around`.
    fn zoom_by(&mut self, factor: f32, around: Point<Pixels>) {
        let zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let (x, y) = self.to_drawing(around);
        let local = around - self.bounds.get().origin;
        self.offset = point(local.x - px(x * zoom), local.y - px(y * zoom));
        self.zoom = zoom;
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
        // Blocks take their own clicks.
        if !self.block_at(event.position) {
            self.panning = Some(event.position);
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(last) = self.panning else {
            return;
        };
        if event.pressed_button != Some(MouseButton::Left) {
            self.panning = None;
            return;
        }
        self.offset += event.position - last;
        self.panning = Some(event.position);
        cx.notify();
    }

    fn scroll(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(16.));
        if event.modifiers.secondary() || event.modifiers.control {
            self.zoom_by(1. - f32::from(delta.y) / 300., event.position);
        } else {
            self.offset += delta;
        }
        cx.notify();
    }

    fn pinch(&mut self, event: &PinchEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.zoom_by(1. + event.delta, event.position);
        cx.notify();
    }

    /// The list's sections in the order the file has them, each with its
    /// functions' indices; the top of the file's first.
    fn sections(&self) -> Vec<(Option<String>, Vec<usize>)> {
        let mut sections: Vec<(Option<String>, Vec<usize>)> = Vec::new();
        for (ix, function) in self.functions.iter().enumerate() {
            match sections
                .iter_mut()
                .find(|(section, _)| *section == function.section)
            {
                Some((_, members)) => members.push(ix),
                None => sections.push((function.section.clone(), vec![ix])),
            }
        }
        sections.sort_by_key(|(section, _)| section.is_some());
        sections
    }

    /// Fold section `section` shut, or open it again.
    pub fn toggle_section(&mut self, section: Option<String>, cx: &mut Context<Self>) {
        if !self.folded.remove(&section) {
            self.folded.insert(section);
        }
        cx.notify();
    }

    #[cfg(test)]
    pub fn folded(&self, section: Option<&str>) -> bool {
        self.folded.contains(&section.map(str::to_string))
    }

    fn render_list(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let sections = self.sections();
        // A file of free functions needs no headings.
        let headed = sections.iter().any(|(section, _)| section.is_some());
        let mut rows: Vec<AnyElement> = Vec::new();
        for (number, (section, members)) in sections.into_iter().enumerate() {
            let folded = self.folded.contains(&section);
            if headed {
                let label = section.clone().unwrap_or_else(|| "Free functions".into());
                let toggle = section.clone();
                rows.push(
                    h_flex()
                        .id(("blueprint-section", number))
                        .mt(px(if number == 0 { 0. } else { 6. }))
                        .px_1()
                        .py(px(3.))
                        .gap_1()
                        .rounded_md()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .hover(|style| style.bg(theme.list_hover))
                        .child(
                            Icon::new(if folded {
                                IconName::ChevronRight
                            } else {
                                IconName::ChevronDown
                            })
                            .size(px(12.))
                            .flex_none(),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .child(label),
                        )
                        .child(
                            div()
                                .flex_none()
                                .opacity(0.7)
                                .child(members.len().to_string()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle_section(toggle.clone(), cx)
                        }))
                        .into_any_element(),
                );
            }
            if folded {
                continue;
            }
            for ix in members {
                let current = Some(ix) == self.current;
                let function = &self.functions[ix];
                rows.push(
                    div()
                        .id(("blueprint-function", ix))
                        .when(headed, |this| this.ml(px(14.)))
                        .px_2()
                        .py(px(3.))
                        .rounded_md()
                        .truncate()
                        .text_sm()
                        .font_family(theme.mono_font_family.clone())
                        .when(current, |this| {
                            this.bg(theme.list_active).text_color(theme.foreground)
                        })
                        .when(!current, |this| {
                            this.text_color(theme.foreground.opacity(0.75))
                                .hover(|style| style.bg(theme.list_hover))
                        })
                        .child(function.short.clone())
                        .on_click(cx.listener(move |this, _, _, cx| this.show(ix, cx)))
                        .into_any_element(),
                );
            }
        }
        v_flex()
            .flex_none()
            .w(px(LIST_WIDTH))
            .h_full()
            .border_r_1()
            .border_color(theme.title_bar_border)
            .child(
                div()
                    .px_3()
                    .pt_3()
                    .pb_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("Functions"),
            )
            .child(
                div()
                    .id("blueprint-functions")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px_1()
                    .pb_2()
                    .children(rows),
            )
    }

    /// One block: a header with its title, then its code, then its pin
    /// rows: data inputs on the left, named outputs and data values on the
    /// right.
    fn render_node(
        &self,
        ix: usize,
        node: &Node,
        rect: Rect,
        marks: &OnBlocks,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let zoom = self.zoom;
        let scaled = |value: f32| px(value * zoom);
        let accent = kind_colour(node.kind, cx);
        let at = self.to_canvas((rect.x, rect.y));
        let highlighted = self.highlight == Some(ix);
        let paused = marks.paused == Some(ix);
        let visits = marks.visits.get(ix).copied().unwrap_or_default();
        // Once it has run through here, what it hasn't reached fades.
        let unreached = marks.traced() && visits == 0 && !paused;
        let group: SharedString = format!("blueprint-node-{ix}").into();
        let breakpoint = self.debug.enabled.then(|| {
            let set = marks.breakpoints.contains(&ix);
            let size = scaled(9.);
            div()
                .id(("blueprint-breakpoint", ix))
                .flex_none()
                .size(size)
                .rounded(size)
                .border_1()
                .when(set, |this| this.bg(theme.red).border_color(theme.red))
                .when(!set, |this| {
                    this.border_color(transparent_black())
                        .group_hover(group.clone(), |style| {
                            style.border_color(theme.muted_foreground.opacity(0.6))
                        })
                        .hover(|style| style.bg(theme.red.opacity(0.5)))
                })
                .tooltip(move |window, cx| {
                    let text = if set {
                        "Remove the breakpoint"
                    } else {
                        "Add a breakpoint"
                    };
                    Tooltip::new(text).build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle_breakpoint(ix, cx);
                }))
        });
        let visit_count = (visits > 1).then(|| {
            div()
                .flex_none()
                .px(scaled(4.))
                .rounded(scaled(4.))
                .bg(theme.blue.opacity(0.18))
                .text_color(theme.blue)
                .text_size(scaled(10.5))
                .child(format!("×{visits}"))
        });
        let data = self.layout.data;
        let step = node.callee.map(|_| {
            div()
                .id(("blueprint-step-into", ix))
                .flex_none()
                .px(scaled(5.))
                .rounded(scaled(4.))
                .text_color(theme.muted_foreground)
                .hover(|style| {
                    style
                        .bg(theme.foreground.opacity(0.1))
                        .text_color(theme.foreground)
                })
                .child("Open ›")
                .tooltip(|window, cx| {
                    Tooltip::new("Open the blueprint of the function it calls (⌘-click)")
                        .build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.step_into(ix, cx);
                }))
        });
        let mut right: Vec<(String, Hsla)> = Vec::new();
        if layout::named_outs(node.outs.len()) {
            right.extend(
                node.outs
                    .iter()
                    .map(|label| (label.clone(), theme.muted_foreground)),
            );
        }
        let mut left: Vec<(String, Hsla)> = Vec::new();
        if data {
            right.extend(
                node.values
                    .iter()
                    .map(|name| (name.clone(), data_colour(name, cx))),
            );
            left.extend(
                node.inputs
                    .iter()
                    .map(|name| (name.clone(), data_colour(name, cx))),
            );
        }
        let rows = layout::pin_rows(node, data);
        let label = |side: &[(String, Hsla)], row: usize| {
            div()
                .truncate()
                .when_some(side.get(row), |this, (text, colour)| {
                    this.text_color(*colour).child(text.clone())
                })
        };
        let pins = (rows > 0).then(|| {
            v_flex().children((0..rows).map(|row| {
                h_flex()
                    .h(scaled(layout::PIN_HEIGHT))
                    .justify_between()
                    .gap(scaled(8.))
                    .pl(scaled(12.))
                    .pr(scaled(12.))
                    .child(label(&left, row))
                    .child(label(&right, row))
            }))
        });
        let body = (!node.code.is_empty() || pins.is_some()).then(|| {
            v_flex()
                .py(scaled(layout::BODY_PADDING))
                .children(node.code.iter().map(|line| {
                    div()
                        .h(scaled(layout::LINE_HEIGHT))
                        .px(scaled(10.))
                        .font_family(theme.mono_font_family.clone())
                        .text_color(theme.foreground.opacity(0.85))
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(line.clone())
                }))
                .children(pins)
        });
        div()
            .id(("blueprint-node", ix))
            .absolute()
            .left(at.x)
            .top(at.y)
            .w(scaled(rect.width))
            .h(scaled(rect.height))
            .overflow_hidden()
            .rounded(scaled(7.))
            .group(group)
            .border_1()
            .border_color(if paused {
                theme.blue
            } else if highlighted {
                accent
            } else {
                theme.border
            })
            .when(highlighted || paused, |this| this.border_2())
            .when(unreached, |this| this.opacity(0.55))
            .bg(theme.popover)
            .shadow_sm()
            .cursor_pointer()
            .text_size(scaled(11.5))
            .line_height(scaled(layout::LINE_HEIGHT))
            .hover(move |style| style.border_color(accent))
            .child(
                h_flex()
                    .h(scaled(layout::HEADER_HEIGHT))
                    .gap(scaled(6.))
                    .justify_between()
                    .pl(scaled(14.))
                    .pr(scaled(if step.is_some() { 8. } else { 16. }))
                    .bg(accent.opacity(0.2))
                    .border_b_1()
                    .border_color(accent.opacity(0.35))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap(scaled(6.))
                            .children(breakpoint)
                            .when(paused, |this| {
                                this.child(div().flex_none().text_color(theme.blue).child("▶"))
                            })
                            .child(div().truncate().child(node.title.clone())),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap(scaled(4.))
                            .children(visit_count)
                            .children(step),
                    ),
            )
            .children(body)
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                if event.modifiers().secondary() && this.graph.nodes[ix].callee.is_some() {
                    this.step_into(ix, cx);
                } else {
                    this.open_node(ix, cx);
                }
            }))
            .into_any_element()
    }

    /// The callers' blocks, left of the start, with "Called from" above;
    /// while they're found, or when there are none, just the label.
    fn render_callers(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        if self.graph.nodes.is_empty() {
            return Vec::new();
        }
        let theme = cx.theme();
        let zoom = self.zoom;
        let scaled = |value: f32| px(value * zoom);
        let label = match &self.callers {
            None => "Finding callers…",
            Some(callers) if callers.is_empty() => "Not called in this project",
            Some(_) => "Called from",
        };
        let at = self.to_canvas((-(CALLER_GAP + CALLER_WIDTH), -LABEL_HEIGHT));
        let mut elements = vec![
            div()
                .absolute()
                .left(at.x)
                .top(at.y)
                .w(scaled(CALLER_WIDTH))
                .h(scaled(LABEL_HEIGHT))
                .truncate()
                .text_size(scaled(11.))
                .text_color(theme.muted_foreground)
                .child(label)
                .into_any_element(),
        ];
        for (ix, caller) in self.callers.iter().flatten().enumerate() {
            let rect = Self::caller_rect(ix);
            let at = self.to_canvas((rect.x, rect.y));
            let open = caller.clone();
            elements.push(
                v_flex()
                    .id(("blueprint-caller", ix))
                    .absolute()
                    .left(at.x)
                    .top(at.y)
                    .w(scaled(rect.width))
                    .h(scaled(rect.height))
                    .overflow_hidden()
                    .rounded(scaled(7.))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover.opacity(0.85))
                    .cursor_pointer()
                    .text_size(scaled(11.5))
                    .line_height(scaled(layout::LINE_HEIGHT))
                    .hover(|style| style.border_color(theme.muted_foreground))
                    .child(
                        h_flex()
                            .h(scaled(layout::HEADER_HEIGHT))
                            .px(scaled(12.))
                            .bg(theme.muted_foreground.opacity(0.12))
                            .border_b_1()
                            .border_color(theme.border)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.foreground)
                            .child(div().truncate().child(caller.name.clone())),
                    )
                    .child(
                        div()
                            .py(scaled(layout::BODY_PADDING))
                            .px(scaled(12.))
                            .h(scaled(layout::LINE_HEIGHT + 2. * layout::BODY_PADDING))
                            .truncate()
                            .font_family(theme.mono_font_family.clone())
                            .text_color(theme.muted_foreground)
                            .child(caller.place.clone()),
                    )
                    .tooltip(|window, cx| {
                        Tooltip::new("Open its blueprint at the call").build(window, cx)
                    })
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(BlueprintEvent::OpenCaller(open.clone()))
                    }))
                    .into_any_element(),
            );
        }
        elements
    }

    /// The wires from each caller into the start block.
    fn caller_wires(&self) -> Vec<(Point<Pixels>, Point<Pixels>)> {
        if self.graph.nodes.is_empty() {
            return Vec::new();
        }
        let entry = self.to_canvas(self.layout.in_pin(0));
        (0..self.caller_count())
            .map(|ix| {
                let rect = Self::caller_rect(ix);
                let from = self.to_canvas((rect.right(), rect.y + layout::HEADER_HEIGHT / 2.));
                (from, entry)
            })
            .collect()
    }

    /// Each wire's ends in the canvas.
    /// Each wire's ends in the canvas, and whether the debugger went
    /// along it.
    fn wires(&self, marks: &OnBlocks) -> Vec<(Point<Pixels>, Point<Pixels>, bool)> {
        self.graph
            .edges
            .iter()
            .enumerate()
            .map(|(ix, edge)| {
                (
                    self.to_canvas(self.layout.out_pin(&self.graph, edge.from, edge.out)),
                    self.to_canvas(self.layout.in_pin(edge.to)),
                    marks.taken.contains(&ix),
                )
            })
            .collect()
    }

    /// Each data wire's ends in the canvas, with its colour.
    fn data_wires(&self, cx: &App) -> Vec<(Point<Pixels>, Point<Pixels>, Hsla)> {
        if !self.layout.data {
            return Vec::new();
        }
        self.graph
            .data
            .iter()
            .map(|data| {
                let name = &self.graph.nodes[data.from].values[data.value];
                (
                    self.to_canvas(self.layout.value_pin(&self.graph, data.from, data.value)),
                    self.to_canvas(self.layout.input_pin(&self.graph, data.to, data.input)),
                    data_colour(name, cx),
                )
            })
            .collect()
    }

    /// Every data pin, with its colour.
    fn data_pins(&self, cx: &App) -> Vec<(Point<Pixels>, Hsla)> {
        if !self.layout.data {
            return Vec::new();
        }
        let mut pins = Vec::new();
        for (ix, node) in self.graph.nodes.iter().enumerate() {
            for (value, name) in node.values.iter().enumerate() {
                let at = self.layout.value_pin(&self.graph, ix, value);
                pins.push((self.to_canvas(at), data_colour(name, cx)));
            }
            for (input, name) in node.inputs.iter().enumerate() {
                let at = self.layout.input_pin(&self.graph, ix, input);
                pins.push((self.to_canvas(at), data_colour(name, cx)));
            }
        }
        pins
    }

    /// Every exec pin: in on each block but the entry, out on each output,
    /// and whether a wire takes it.
    fn pins(&self) -> Vec<(Point<Pixels>, bool)> {
        let mut pins = Vec::new();
        for (ix, node) in self.graph.nodes.iter().enumerate() {
            if node.kind != Kind::Entry {
                let wired = self.graph.edges.iter().any(|edge| edge.to == ix);
                pins.push((self.to_canvas(self.layout.in_pin(ix)), wired));
            }
            for out in 0..node.outs.len() {
                let wired = self
                    .graph
                    .edges
                    .iter()
                    .any(|edge| edge.from == ix && edge.out == out);
                pins.push((
                    self.to_canvas(self.layout.out_pin(&self.graph, ix, out)),
                    wired,
                ));
            }
        }
        pins
    }

    fn render_canvas(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let zoom = self.zoom;
        let offset = self.offset;
        let grid = theme.foreground.opacity(0.045);
        let wire = theme.foreground.opacity(0.6);
        let pin = theme.foreground.opacity(0.8);
        let marks = debug::on_blocks(&self.graph, self.function_range(), &self.debug);
        let wires = self.wires(&marks);
        let taken_wire = theme.blue;
        let caller_wires = self.caller_wires();
        let caller_wire = theme.muted_foreground.opacity(0.5);
        let pins = self.pins();
        let data_wires = self.data_wires(cx);
        let data_pins = self.data_pins(cx);
        let bounds = self.bounds.clone();
        let behind = canvas(
            move |drawn, _, _| bounds.set(drawn),
            move |drawn, _, window, _| {
                paint_grid(drawn, offset, zoom, grid, window);
                for (from, to, colour) in data_wires {
                    let width = (1.2 * zoom).max(0.75);
                    paint_curve(
                        drawn.origin + from,
                        drawn.origin + to,
                        zoom,
                        width,
                        colour,
                        window,
                    );
                }
                for (from, to) in caller_wires {
                    let width = (1.5 * zoom).max(1.);
                    paint_curve(
                        drawn.origin + from,
                        drawn.origin + to,
                        zoom,
                        width,
                        caller_wire,
                        window,
                    );
                }
                // The way it went on top, brighter and wider.
                let (taken, others): (Vec<_>, Vec<_>) =
                    wires.into_iter().partition(|(_, _, taken)| *taken);
                for (from, to, _) in others {
                    paint_wire(drawn.origin + from, drawn.origin + to, zoom, wire, window);
                }
                for (from, to, _) in taken {
                    let width = (3. * zoom).max(1.5);
                    paint_curve(
                        drawn.origin + from,
                        drawn.origin + to,
                        zoom,
                        width,
                        taken_wire,
                        window,
                    );
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let over = canvas(
            |_, _, _| {},
            move |drawn, _, window, _| {
                for (at, wired) in pins {
                    paint_pin(drawn.origin + at, zoom, pin, wired, window);
                }
                for (at, colour) in data_pins {
                    paint_data_pin(drawn.origin + at, zoom, colour, window);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let nodes: Vec<AnyElement> = self
            .graph
            .nodes
            .iter()
            .zip(&self.layout.rects)
            .enumerate()
            .map(|(ix, (node, rect))| self.render_node(ix, node, *rect, &marks, cx))
            .collect();
        div()
            .id("blueprint-canvas")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.panning = None),
            )
            .on_scroll_wheel(cx.listener(Self::scroll))
            .on_pinch(cx.listener(Self::pinch))
            .child(behind)
            .children(self.render_callers(cx))
            .children(nodes)
            .child(over)
            .child(self.render_hint(cx))
    }

    /// Above the canvas: the function's name, and the data wires' switch.
    fn render_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let name = self
            .current_function()
            .map(|function| function.name.clone())
            .unwrap_or_default();
        h_flex()
            .flex_none()
            .h(px(HEADER_HEIGHT))
            .px_3()
            .gap_3()
            .justify_between()
            .border_b_1()
            .border_color(theme.title_bar_border)
            .child(
                div()
                    .truncate()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(name),
            )
            .child(
                Switch::new("blueprint-data-wires")
                    .checked(self.data_wires)
                    .small()
                    .label("Data wires")
                    .tooltip("Wires from each let and parameter to the blocks reading it")
                    .on_click(cx.listener(|this, on: &bool, _, cx| this.set_data_wires(*on, cx))),
            )
    }

    fn render_hint(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let text = if self.functions.is_empty() {
            "No functions in this file."
        } else {
            "Click a block to go to its code · ⌘-click a call to open what it calls · drag or scroll to move · ⌘-scroll or pinch to zoom · esc back to the code"
        };
        div()
            .absolute()
            .bottom(px(10.))
            .left(px(14.))
            .right(px(14.))
            .truncate()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(text)
    }
}

impl Render for BlueprintView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .key_context("Blueprint")
            .track_focus(&self.focus)
            .size_full()
            .items_stretch()
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(BlueprintEvent::Close);
                    cx.stop_propagation();
                }
            }))
            .child(self.render_list(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(self.render_header(cx))
                    .child(self.render_canvas(cx)),
            )
    }
}

/// Test modules, folded shut until opened.
fn is_test_section(section: &str) -> bool {
    section
        .split(" › ")
        .any(|part| part == "mod tests" || part == "mod test")
}

/// A block's colour, by what it does: the start red as in Unreal, calls
/// blue, bindings green, branching yellow, exits magenta.
fn kind_colour(kind: Kind, cx: &App) -> Hsla {
    let theme = cx.theme();
    match kind {
        Kind::Entry => theme.red,
        Kind::Call => theme.blue,
        Kind::Let => theme.green,
        Kind::Branch | Kind::Match | Kind::Loop => theme.yellow,
        Kind::Exit => theme.magenta,
        Kind::Statement => theme.muted_foreground,
    }
}

/// A data wire's colour, by the name it carries, so a name keeps its
/// colour wherever it's read.
fn data_colour(name: &str, cx: &App) -> Hsla {
    let theme = cx.theme();
    let palette = [
        theme.cyan,
        theme.green,
        theme.magenta,
        theme.yellow,
        theme.blue,
    ];
    let hash = name.bytes().fold(0usize, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(byte as usize)
    });
    palette[hash % palette.len()]
}

fn paint_grid(
    bounds: Bounds<Pixels>,
    offset: Point<Pixels>,
    zoom: f32,
    colour: Hsla,
    window: &mut Window,
) {
    let step = GRID * zoom;
    // Too dense to help.
    if step < 8. {
        return;
    }
    let first = |origin: Pixels, offset: Pixels| {
        let offset = f32::from(offset).rem_euclid(step);
        f32::from(origin) + offset
    };
    let mut x = first(bounds.origin.x, offset.x);
    while x < f32::from(bounds.right()) {
        window.paint_quad(fill(
            Bounds::new(
                point(px(x), bounds.origin.y),
                size(px(1.), bounds.size.height),
            ),
            colour,
        ));
        x += step;
    }
    let mut y = first(bounds.origin.y, offset.y);
    while y < f32::from(bounds.bottom()) {
        window.paint_quad(fill(
            Bounds::new(
                point(bounds.origin.x, px(y)),
                size(bounds.size.width, px(1.)),
            ),
            colour,
        ));
        y += step;
    }
}

/// An exec wire: leaving its output to the right and coming into the next
/// block from the left, as in Unreal.
fn paint_wire(
    from: Point<Pixels>,
    to: Point<Pixels>,
    zoom: f32,
    colour: Hsla,
    window: &mut Window,
) {
    paint_curve(from, to, zoom, (2. * zoom).max(1.), colour, window);
}

/// A wire `width` wide, out to the right and in from the left.
fn paint_curve(
    from: Point<Pixels>,
    to: Point<Pixels>,
    zoom: f32,
    width: f32,
    colour: Hsla,
    window: &mut Window,
) {
    let reach = px((f32::from(to.x - from.x).abs() / 2.).max(40. * zoom));
    let mut path = PathBuilder::stroke(px(width));
    path.move_to(from);
    path.cubic_bezier_to(to, point(from.x + reach, from.y), point(to.x - reach, to.y));
    if let Ok(path) = path.build() {
        window.paint_path(path, colour);
    }
}

/// An exec pin, an arrowhead pointing the way the flow goes: filled when
/// a wire takes it, an outline when nothing does.
fn paint_pin(at: Point<Pixels>, zoom: f32, colour: Hsla, wired: bool, window: &mut Window) {
    let half = px(PIN_SIZE * zoom / 2.);
    let left = at.x - half;
    let mut path = if wired {
        PathBuilder::fill()
    } else {
        PathBuilder::stroke(px((1.3 * zoom).max(1.)))
    };
    path.move_to(point(left, at.y - half));
    path.line_to(point(at.x + half, at.y));
    path.line_to(point(left, at.y + half));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, colour);
    }
}

/// A data pin, a dot as in Unreal.
fn paint_data_pin(at: Point<Pixels>, zoom: f32, colour: Hsla, window: &mut Window) {
    let half = px(DATA_PIN_SIZE * zoom / 2.);
    window.paint_quad(
        fill(
            Bounds::new(point(at.x - half, at.y - half), size(half * 2., half * 2.)),
            colour,
        )
        .corner_radii(half),
    );
}
