//! Drawing the agent's drawings over the level showing, in the agent's
//! colour: each box a place lands on outlined; a trace's stops numbered
//! with their step ranges and joined by one dashed path; a note or a
//! one-step stop's words under its box; a group framed with its label;
//! ghosts beside the place they go by; proposed purposes and labels to
//! keep or not. Boxes out of view get an arrow at the canvas edge that
//! flies there. A list in the corner names every drawing and its steps,
//! greying those this level can't show; a tour shows where it is below.

use std::collections::HashMap;

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_commands::surface::agent_accent;

use super::super::layout::Rect;
use super::super::lens::{Drawing, Kind, Place};
use super::draw::{Line, View};
use super::lens::{Landing, Source, Stop, fold};
use super::{MapEvent, MapView};

/// Most lines of words under one box before "+N more".
const MOST_WORDS: usize = 3;
/// How far from the canvas edge an arrow to a box out of view sits.
const EDGE: f32 = 22.;
/// A ghost box's size on screen.
const GHOST: (f32, f32) = (190., 54.);

/// What the user chose for a proposal's ghosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProposalChoice {
    MakeTask,
    Implement,
    Dismiss,
}

/// A box the agent's drawings land on, on screen.
pub(super) struct Marked {
    pub(super) key: String,
    /// In the drawing, to fly to.
    pub(super) drawn: Rect,
    pub(super) rect: Rect,
    /// Lines of words under it.
    pub(super) words: Vec<String>,
    /// Step ranges, top left.
    pub(super) badges: Vec<String>,
    /// The tour's step is here.
    pub(super) toured: bool,
    /// Its words are proposed, to keep or not.
    pub(super) proposed: bool,
    /// Only pins land here: drawn fainter.
    pub(super) pinned: bool,
    /// A frame holding other boxes (a Plan's type or file): its words go
    /// above it, clear of theirs.
    pub(super) above: bool,
}

/// A path along a trace, or an arrow lit, labelled or proposed.
pub(super) struct Path {
    pub(super) from: (f32, f32),
    pub(super) to: (f32, f32),
    pub(super) dashed: bool,
    pub(super) width: f32,
    pub(super) pinned: bool,
}

/// Words on an arrow, at its middle.
pub(super) struct Tag {
    pub(super) at: (f32, f32),
    pub(super) text: String,
}

/// Places circled together.
pub(super) struct Group {
    pub(super) rect: Rect,
    pub(super) label: String,
    pub(super) pinned: bool,
}

/// A box the agent proposes.
pub(super) struct GhostBox {
    pub(super) rect: Rect,
    pub(super) name: String,
    pub(super) purpose: String,
}

/// Everything the agent drew at the level showing, on screen: what the
/// canvas draws, and Export Image.
#[derive(Default)]
pub(super) struct Picture {
    pub(super) boxes: Vec<Marked>,
    pub(super) paths: Vec<Path>,
    pub(super) tags: Vec<Tag>,
    pub(super) groups: Vec<Group>,
    pub(super) ghosts: Vec<GhostBox>,
    /// The conversation's proposals, by id, and their first ghost: where
    /// what to do with them goes.
    pub(super) proposals: Vec<(usize, Rect)>,
}

impl Picture {
    fn mark(
        &mut self,
        landing: &Landing,
        screen: &dyn Fn(Rect) -> Rect,
        pinned: bool,
    ) -> &mut Marked {
        let at = match self.boxes.iter().position(|b| b.key == landing.key) {
            Some(at) => at,
            None => {
                self.boxes.push(Marked {
                    key: landing.key.clone(),
                    drawn: landing.rect,
                    rect: screen(landing.rect),
                    words: Vec::new(),
                    badges: Vec::new(),
                    toured: false,
                    proposed: false,
                    pinned: true,
                    above: landing.key.starts_with("frame:"),
                });
                self.boxes.len() - 1
            }
        };
        let marked = &mut self.boxes[at];
        marked.pinned &= pinned;
        marked
    }
}

impl MapView {
    /// The agent's drawings at the level showing, `screen` putting a rect of
    /// the drawing on screen.
    pub(super) fn lens_picture(&self, screen: &dyn Fn(Rect) -> Rect) -> Picture {
        let mut picture = Picture::default();
        let tour = self.lens.tour;
        for (source, drawing) in self.drawings_showing() {
            let pinned = matches!(source, Source::Pin(_));
            let landings: Vec<Option<Landing>> = drawing
                .steps
                .iter()
                .map(|step| self.landing(&step.place))
                .collect();
            match drawing.kind {
                Kind::Trace | Kind::Tour => {
                    let stops = fold(&landings);
                    let toured = tour.filter(|t| t.source == source).map(|t| t.step);
                    for stop in &stops {
                        let marked = picture.mark(&stop.landing, screen, pinned);
                        marked.badges.push(stop.range());
                        marked.toured |= toured.is_some_and(|step| stop.steps.contains(&step));
                        let words = match stop.steps.as_slice() {
                            [step] if drawing.steps[*step].label.is_empty() => continue,
                            [step] => format!("{} {}", step + 1, drawing.steps[*step].label),
                            _ => stop_words(drawing, stop),
                        };
                        marked.words.push(words);
                    }
                    for pair in stops.windows(2) {
                        let (a, b) = (screen(pair[0].landing.rect), screen(pair[1].landing.rect));
                        // A box and the frame it's in: one place, no path.
                        if holds(a, b) || holds(b, a) {
                            continue;
                        }
                        let (from, to) = super::scene::edge_points(a, b);
                        picture.paths.push(Path {
                            from,
                            to,
                            dashed: true,
                            width: 2.5,
                            pinned,
                        });
                    }
                }
                Kind::Highlight | Kind::Group => {
                    for landing in landings.iter().flatten() {
                        picture.mark(landing, screen, pinned);
                    }
                    let hull = landings
                        .iter()
                        .flatten()
                        .map(|l| screen(l.rect))
                        .reduce(|a, b| a.union(&b));
                    if let (Kind::Group, Some(hull)) = (drawing.kind, hull) {
                        let pad = 16.;
                        picture.groups.push(Group {
                            rect: Rect {
                                x: hull.x - pad,
                                y: hull.y - pad,
                                width: hull.width + pad * 2.,
                                height: hull.height + pad * 2.,
                            },
                            label: drawing.title.clone(),
                            pinned,
                        });
                    }
                }
                Kind::Note => {
                    for (step, landing) in drawing.steps.iter().zip(&landings) {
                        if let Some(landing) = landing {
                            picture
                                .mark(landing, screen, pinned)
                                .words
                                .push(step.label.clone());
                        }
                    }
                }
                Kind::Describe => {
                    for (step, landing) in drawing.steps.iter().zip(&landings) {
                        if let Some(landing) = landing {
                            let marked = picture.mark(landing, screen, pinned);
                            marked.words.push(format!("Purpose: {}", step.label));
                            marked.proposed |= !pinned;
                        }
                    }
                }
                Kind::Propose => self.picture_ghosts(&mut picture, source, drawing, screen),
            }
            // Arrows lit or labelled between real places.
            if drawing.kind != Kind::Propose {
                for link in &drawing.links {
                    let (Some(a), Some(b)) = (self.landing(&link.from), self.landing(&link.to))
                    else {
                        continue;
                    };
                    if a.key == b.key {
                        continue;
                    }
                    picture.mark(&a, screen, pinned);
                    picture.mark(&b, screen, pinned);
                    let (from, to) = super::scene::edge_points(screen(a.rect), screen(b.rect));
                    picture.paths.push(Path {
                        from,
                        to,
                        dashed: drawing.kind != Kind::Highlight,
                        width: 3.,
                        pinned,
                    });
                    if !link.label.is_empty() {
                        picture.tags.push(Tag {
                            at: ((from.0 + to.0) / 2., (from.1 + to.1) / 2.),
                            text: link.label.clone(),
                        });
                    }
                }
            }
        }
        picture
    }

    /// A proposal's ghosts, each beside the place it goes by, stacked, and
    /// the arrows between them and real places.
    fn picture_ghosts(
        &self,
        picture: &mut Picture,
        source: Source,
        drawing: &Drawing,
        screen: &dyn Fn(Rect) -> Rect,
    ) {
        let mut placed: HashMap<String, Rect> = HashMap::new();
        let mut stacked: HashMap<String, usize> = HashMap::new();
        for ghost in &drawing.ghosts {
            let Some(near) = self.landing(&ghost.near) else {
                continue;
            };
            picture.mark(&near, screen, matches!(source, Source::Pin(_)));
            let near = screen(near.rect);
            let n = stacked.entry(ghost.near.clone()).or_default();
            let rect = Rect {
                x: near.right() + 36.,
                y: near.y + *n as f32 * (GHOST.1 + 12.),
                width: GHOST.0,
                height: GHOST.1,
            };
            *n += 1;
            placed.insert(ghost.name.clone(), rect);
            let (from, to) = super::scene::edge_points(near, rect);
            picture.paths.push(Path {
                from,
                to,
                dashed: true,
                width: 1.5,
                pinned: true,
            });
            picture.ghosts.push(GhostBox {
                rect,
                name: ghost.name.clone(),
                purpose: ghost.purpose.clone(),
            });
        }
        let end = |name: &str| -> Option<Rect> {
            placed
                .get(name)
                .copied()
                .or_else(|| self.landing(name).map(|l| screen(l.rect)))
        };
        for link in &drawing.links {
            if let (Some(a), Some(b)) = (end(&link.from), end(&link.to)) {
                let (from, to) = super::scene::edge_points(a, b);
                picture.paths.push(Path {
                    from,
                    to,
                    dashed: true,
                    width: 2.,
                    pinned: false,
                });
                if !link.label.is_empty() {
                    picture.tags.push(Tag {
                        at: ((from.0 + to.0) / 2., (from.1 + to.1) / 2.),
                        text: link.label.clone(),
                    });
                }
            }
        }
        if let (Source::Conversation(id), Some(first)) = (
            source,
            drawing.ghosts.iter().find_map(|g| placed.get(&g.name)),
        ) {
            picture.proposals.push((id, *first));
        }
    }

    /// Everything the agent drew, over the scene seen through `view`.
    pub(super) fn render_lens(&self, view: View, cx: &Context<Self>) -> Vec<AnyElement> {
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
        let mut edges = Vec::new();
        for marked in &picture.boxes {
            let rect = marked.rect;
            let colour = colour(marked.pinned);
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
                        .into_any_element(),
                );
            }
            if !marked.words.is_empty() {
                words.push(
                    v_flex()
                        .absolute()
                        .left(px(rect.x))
                        .when(marked.above, |this| this.bottom(px(size.1 - rect.y + 22.)))
                        .when(!marked.above, |this| this.top(px(rect.bottom() + 8.)))
                        .max_w(px(rect.width.max(240.)))
                        .px_2()
                        .py_1()
                        .gap_0p5()
                        .rounded(px(6.))
                        .bg(theme.background.opacity(0.94))
                        .border_1()
                        .border_color(colour.opacity(0.6))
                        .when(marked.proposed, |this| this.border_dashed())
                        .text_xs()
                        .text_color(theme.foreground)
                        .children(
                            capped(&marked.words)
                                .into_iter()
                                .map(|line| div().child(line)),
                        )
                        .into_any_element(),
                );
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
        out.extend(edges);
        out.extend(self.render_legend(cx));
        out.extend(self.render_tour_bar(cx));
        out
    }

    /// What to do with a proposal, over its first ghost.
    fn render_proposal_buttons(&self, id: usize, first: Rect, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let agent = agent_accent(cx);
        let choices = [
            (ProposalChoice::MakeTask, "Make Task"),
            (ProposalChoice::Implement, "Implement"),
            (ProposalChoice::Dismiss, "Dismiss"),
        ];
        h_flex()
            .absolute()
            .left(px(first.x))
            .top(px(first.y - 30.))
            .gap_1()
            .children(choices.into_iter().enumerate().map(|(n, (choice, label))| {
                h_flex()
                    .id(("map-proposal", id * 4 + n))
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
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(MapEvent::Proposal(id, choice));
                    }))
            }))
            .into_any_element()
    }

    /// An arrow at the canvas edge towards box `key`, out of view: a click
    /// flies there.
    fn render_edge_arrow(
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

    /// The list of drawings in the corner: each with its steps, those this
    /// level can't show greyed; a step clicked flies there.
    fn render_legend(&self, cx: &Context<Self>) -> Option<AnyElement> {
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
            let header =
                h_flex()
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
                        this.child(legend_button(("map-tour", n), "Tour", cx).on_click(
                            cx.listener(move |view, _, _, cx| view.start_tour(source, cx)),
                        ))
                    })
                    .when(drawing.kind == Kind::Describe && pinned.is_none(), |this| {
                        this.child(legend_button(("map-keep", n), "Keep", cx).on_click(
                            cx.listener(move |view, _, _, cx| {
                                view.settle_descriptions(source, true, cx)
                            }),
                        ))
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
                    .truncate()
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
        Some(
            v_flex()
                .id("map-lens-list")
                .debug_selector(|| "map-lens-list".into())
                .absolute()
                .left(px(12.))
                .bottom(px(32.))
                .w(px(330.))
                .max_h(relative(0.45))
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

    /// While touring: the step showing, and the keys that move on.
    fn render_tour_bar(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let tour = self.lens.tour?;
        let drawing = self.drawing(tour.source)?;
        let step = drawing.steps.get(tour.step)?;
        let agent = agent_accent(cx);
        Some(
            h_flex()
                .id("map-tour-bar")
                .debug_selector(|| "map-tour-bar".into())
                .absolute()
                .bottom(px(32.))
                .left(px(360.))
                .right(px(12.))
                .justify_center()
                .child(
                    h_flex()
                        .gap_2()
                        .px_3()
                        .py_1p5()
                        .rounded_full()
                        .bg(agent)
                        .text_sm()
                        .text_color(white())
                        .shadow_md()
                        .child(format!("{} of {}", tour.step + 1, drawing.steps.len()))
                        .child(
                            div()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(step.label.clone()),
                        )
                        .child(div().opacity(0.8).child(self.short_name(&step.place)))
                        .child(div().opacity(0.8).child("← → · Esc")),
                )
                .into_any_element(),
        )
    }

    /// A place as the list names it: a function's title, a file's name, a
    /// folder's last part, a domain's name.
    pub(super) fn short_name(&self, name: &str) -> String {
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
            None => name.to_string(),
        }
    }
}

/// A stop's words when several steps land on its box: the first and last
/// labels.
fn stop_words(drawing: &Drawing, stop: &Stop) -> String {
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

fn file_name(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

/// Where on the canvas edge the arrow to a box out of view goes, and
/// which way it points.
pub(super) fn edge_spot(on_screen: Rect, (width, height): (f32, f32)) -> (f32, f32, &'static str) {
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

/// Whether `outer` holds all of `inner`.
fn holds(outer: Rect, inner: Rect) -> bool {
    outer.x <= inner.x + 0.5
        && outer.y <= inner.y + 0.5
        && outer.right() + 0.5 >= inner.right()
        && outer.bottom() + 0.5 >= inner.bottom()
}

/// Words under a box, at most a few lines.
pub(super) fn capped(words: &[String]) -> Vec<String> {
    let mut lines: Vec<String> = words.iter().take(MOST_WORDS).cloned().collect();
    if words.len() > MOST_WORDS {
        lines.push(format!("+{} more", words.len() - MOST_WORDS));
    }
    lines
}

fn legend_button(id: (&'static str, usize), label: &'static str, cx: &App) -> Stateful<Div> {
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
