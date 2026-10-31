//! Drawing the Map: the header with where you are and its buttons, then
//! each scene showing (the level left fading out under the one coming
//! in): frames behind their boxes, Plan's calls as curves from a box's
//! right to another's left (dashed for guesses and callbacks), Site's as
//! bowed arrows over the boxes, thicker with more calls. Plan's text
//! scales with the view; Site's stays one size, so it reads at any zoom.

use std::time::Instant;

use gpui_kit::component::switch::Switch;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::super::layout::{HEADER_HEIGHT, PADDING, ROW_HEIGHT, Rect};
use super::super::plan::Focus;
use super::super::site::Show;
use super::input::Hover;
use super::scene::{Arrow, Card, FrameBox, Look, Scene};
use super::zoom::Camera;
use super::{Expand, Level, MapView, domain};

const BAR_HEIGHT: f32 = 34.;
/// A link's words kept where they were stay within this many points of
/// it, as zooming in spreads them apart.
const STICKY_REACH: f32 = 220.;
const GRID: f32 = 24.;

/// Where a scene goes on a canvas `size` big, seen by `camera`.
#[derive(Clone, Copy)]
pub(super) struct View {
    pub(super) camera: Camera,
    pub(super) size: (f32, f32),
}

impl View {
    pub(super) fn point(&self, (x, y): (f32, f32)) -> Point<Pixels> {
        point(
            px((x - self.camera.x) * self.camera.zoom + self.size.0 / 2.),
            px((y - self.camera.y) * self.camera.zoom + self.size.1 / 2.),
        )
    }

    pub(super) fn scaled(&self, value: f32) -> Pixels {
        px(value * self.camera.zoom)
    }
}

/// What's lit over a scene: selected, under the pointer, on the paused
/// program's stack, or just fired.
pub(super) struct Lit {
    pub(super) selected: Vec<String>,
    pub(super) hovered: Option<String>,
    path: Vec<String>,
}

impl MapView {
    /// Where you are: the project, the level, what it's about.
    fn crumbs(&self) -> Vec<String> {
        let project = self
            .root
            .file_name()
            .map_or("Project".into(), |name| name.to_string_lossy().into_owned());
        let mut crumbs = vec![project];
        match &self.level {
            Level::Site | Level::Domain => {}
            Level::Plan(Focus::Module(place)) => crumbs.push(place.trim_end_matches('/').into()),
            Level::Plan(Focus::Function(symbol)) => {
                crumbs.push(symbol.file.clone());
                crumbs.push(symbol.title());
            }
        }
        crumbs
    }

    pub(super) fn render_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let crumbs = self.crumbs();
        let last = crumbs.len() - 1;
        let mut trail: Vec<AnyElement> = Vec::new();
        for (ix, label) in crumbs.into_iter().enumerate() {
            if ix > 0 {
                trail.push(
                    div()
                        .text_color(theme.muted_foreground)
                        .child("›")
                        .into_any_element(),
                );
            }
            trail.push(
                div()
                    .px_1()
                    .truncate()
                    .text_color(if ix == last {
                        theme.foreground
                    } else {
                        theme.muted_foreground
                    })
                    .child(label)
                    .into_any_element(),
            );
        }
        let button = |id: &'static str, label: &'static str, tip: &'static str| {
            h_flex()
                .id(id)
                .debug_selector(move || id.into())
                .flex_none()
                .h(px(22.))
                .px_2()
                .gap_1()
                .rounded(px(5.))
                .text_xs()
                .text_color(theme.foreground.opacity(0.85))
                .hover(|style| style.bg(theme.foreground.opacity(0.08)))
                .child(label)
                .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
        };
        let plan = matches!(self.level, Level::Plan(_));
        let tints = (self.has_tints() && self.level != Level::Domain).then(|| {
            h_flex()
                .id("map-tints")
                .gap_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(
                    Switch::new("map-tints-switch")
                        .small()
                        .checked(!self.domains.tints_off)
                        .on_click(cx.listener(|this, on: &bool, _, cx| this.set_tints(*on, cx))),
                )
                .child("Domains")
                .tooltip(|window, cx| {
                    Tooltip::new("Tint each box with its domain's colour").build(window, cx)
                })
        });
        let site = self.level == Level::Site;
        let tests = site.then(|| {
            h_flex()
                .id("map-tests")
                .gap_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(
                    Switch::new("map-tests-switch")
                        .small()
                        .checked(self.site_show.tests)
                        .on_click(cx.listener(|this, on: &bool, _, cx| {
                            this.set_site_show(Show { tests: *on }, cx)
                        })),
                )
                .child("Tests")
                .tooltip(|window, cx| {
                    Tooltip::new("Show the test files, and the calls they make").build(window, cx)
                })
        });
        let pinned = (!self.lens.pins.is_empty()).then(|| {
            h_flex()
                .id("map-pinned")
                .gap_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(
                    Switch::new("map-pinned-switch")
                        .small()
                        .checked(self.lens.pins_shown)
                        .on_click(
                            cx.listener(|this, on: &bool, _, cx| this.set_pins_shown(*on, cx)),
                        ),
                )
                .child("Pinned")
                .tooltip(|window, cx| {
                    Tooltip::new("Show the agent's drawings pinned to the Map").build(window, cx)
                })
        });
        let agent = jig_commands::surface::agent_accent(cx);
        let explain = self.project.is_some().then(|| {
            button(
                "map-explain",
                "Explain",
                "Ask the agent to explain this level",
            )
            .text_color(agent)
            .child(
                jig_commands::surface::agent_icon()
                    .size(px(11.))
                    .text_color(agent),
            )
            .on_click(cx.listener(|this, _, _, cx| this.explain_level(cx)))
        });
        let domain_buttons = (self.level == Level::Domain).then(|| self.render_domain_buttons(cx));
        let sketches = (!self.sketches.sketches.is_empty()).then(|| {
            button(
                "map-sketches",
                "Sketches",
                "Every sketch, archived ones too",
            )
            .when(self.sketches.list_open, |this| {
                this.bg(theme.foreground.opacity(0.1))
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.sketches.list_open = !this.sketches.list_open;
                cx.notify();
            }))
        });
        let export = self.render_export_button(cx);
        h_flex()
            .flex_none()
            .h(px(BAR_HEIGHT))
            .px_3()
            .gap_3()
            .justify_between()
            .border_b_1()
            .border_color(theme.title_bar_border)
            .child(
                h_flex()
                    .min_w_0()
                    .gap_1()
                    .text_sm()
                    .child(
                        Icon::new(IconName::Map)
                            .size(px(13.))
                            .flex_none()
                            .text_color(theme.muted_foreground),
                    )
                    .children(trail),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .children(explain)
                    .children(sketches)
                    .children(domain_buttons)
                    .children(pinned)
                    .children(tests)
                    .children(tints)
                    .child(export)
                    .when(plan, |this| {
                        this.child(
                            button("map-tidy", "Tidy", "Lay the boxes out afresh")
                                .on_click(cx.listener(|this, _, _, cx| this.tidy(cx))),
                        )
                        .child(
                            h_flex()
                                .id("map-live")
                                .gap_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(
                                    Switch::new("map-live-switch")
                                        .small()
                                        .checked(self.live)
                                        .on_click(cx.listener(|this, on: &bool, _, cx| {
                                            this.set_live(*on, cx)
                                        })),
                                )
                                .child("Live")
                                .tooltip(|window, cx| {
                                    Tooltip::new(
                                        "While debugging, functions glow as the program enters them",
                                    )
                                    .build(window, cx)
                                }),
                        )
                    }),
            )
    }

    pub(super) fn render_canvas(
        &self,
        now: Instant,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let frame = self.frame(now);
        let size = self.canvas_size();
        let view = View {
            camera: frame.camera,
            size,
        };
        let scene = self.scene(frame.camera, now);
        let lit = Lit {
            selected: self.selected.clone(),
            hovered: match &self.hover {
                Some(Hover::Card(key, _)) => Some(key.clone()),
                _ => None,
            },
            path: self.path_keys(),
        };
        let grid = theme.foreground.opacity(0.04);
        let (offset, zoom) = (self.offset, self.zoom);
        let bounds = self.bounds.clone();
        let behind = canvas(
            move |drawn, _, _| bounds.set(drawn),
            move |drawn, _, window, _| paint_grid(drawn, offset, zoom, grid, window),
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        // A drag follows the pointer anywhere in the window until it's
        // let go; hovering is heard over boxes too.
        let dragging = self.drag.is_some().then(|| cx.weak_entity());
        let hovering = self.drag.is_none().then(|| cx.weak_entity());
        let over = canvas(
            |_, _, _| {},
            move |drawn, _, window, _| {
                if let Some(view) = hovering {
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble && drawn.contains(&event.position) {
                            view.update(cx, |view, cx| view.hover_move(event, window, cx))
                                .ok();
                        }
                    });
                }
                if let Some(view) = dragging {
                    let moving = view.clone();
                    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble {
                            moving
                                .update(cx, |view, cx| view.mouse_move(event, window, cx))
                                .ok();
                        }
                    });
                    window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
                        if phase == DispatchPhase::Bubble {
                            view.update(cx, |view, cx| view.mouse_up(event, window, cx))
                                .ok();
                        }
                    });
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();
        let old = frame.old.map(|(scene, camera, opacity)| {
            let view = View { camera, size };
            self.render_scene(scene, view, opacity, false, &lit, now, window, cx)
        });
        let leaving = self.leaving_scene(now).map(|(scene, opacity)| {
            self.render_scene(&scene, view, opacity, false, &lit, now, window, cx)
        });
        let current = self.render_scene(&scene, view, frame.opacity, true, &lit, now, window, cx);
        *self.drawn.borrow_mut() = scene;
        let empty = self.project.is_none();
        div()
            .id("map-canvas")
            .relative()
            .bg(theme::canvas(cx))
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_hidden()
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::other_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::other_down))
            .on_scroll_wheel(cx.listener(Self::scroll))
            .on_pinch(cx.listener(Self::pinch))
            .child(behind)
            .children(old)
            .children(leaving)
            .child(current)
            .child(over)
            .children(self.render_sketches(view, cx))
            .children(self.render_lens(view, cx))
            .children(self.render_marquee(cx))
            .children(self.render_domain_overlay(cx))
            .children(self.render_hover(cx))
            .children(self.render_tool_strip(cx))
            .children(self.render_sketch_list(cx))
            .child(self.render_ruler(window, cx))
            .when(self.tool() != super::sketch::Tool::Pointer, |this| {
                this.cursor_crosshair()
            })
            .when(empty, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(80.))
                        .left(px(0.))
                        .right(px(0.))
                        .flex()
                        .justify_center()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child("Reading the project…"),
                )
            })
            .child(self.render_hint(cx))
    }

    /// Where a link's words go on screen, its top left: where they went
    /// last time, kept in the drawing so they move with it, as long as
    /// that's still clear of `avoid` and near the link; else a spot found
    /// afresh (`label_spot`), kept for next time. Looking again every frame
    /// would have them jump about as the view moves.
    fn sticky_spot(
        &self,
        arrow: &Arrow,
        ends: ((f32, f32), (f32, f32)),
        size: (f32, f32),
        avoid: &[Rect],
        view: View,
    ) -> (f32, f32) {
        let key = format!("{}|{}", arrow.keys.0, arrow.keys.1);
        let zoom = view.camera.zoom.max(0.0001);
        let to_screen = |at: (f32, f32)| {
            let p = view.point(at);
            (f32::from(p.x), f32::from(p.y))
        };
        let to_drawing = |(x, y): (f32, f32)| {
            (
                (x - view.size.0 / 2.) / zoom + view.camera.x,
                (y - view.size.1 / 2.) / zoom + view.camera.y,
            )
        };
        if let Some(&(at, was)) = self.label_spots.borrow().get(&key)
            && was == size
        {
            let (x, y) = to_screen(at);
            let rect = Rect {
                x,
                y,
                width: size.0,
                height: size.1,
            };
            let clear = avoid.iter().all(|other| !rect.overlaps(other));
            let near = leader(ends.0, ends.1, rect)
                .is_none_or(|(on, edge)| (on.0 - edge.0).hypot(on.1 - edge.1) < STICKY_REACH);
            if clear && near {
                return (x, y);
            }
        }
        // Kept on screen when the link's middle is; else wherever's clear.
        let middle = bow_point(
            point(px(ends.0.0), px(ends.0.1)),
            point(px(ends.1.0), px(ends.1.1)),
            0.5,
        );
        let screen = Rect {
            x: 0.,
            y: 0.,
            width: view.size.0,
            height: view.size.1,
        };
        let within = if screen.contains((f32::from(middle.x), f32::from(middle.y))) {
            screen
        } else {
            Rect {
                x: -1e6,
                y: -1e6,
                width: 2e6,
                height: 2e6,
            }
        };
        let spot = label_spot(ends.0, ends.1, size, avoid, within);
        self.label_spots
            .borrow_mut()
            .insert(key, (to_drawing(spot), size));
        spot
    }

    /// The keys of the boxes on the paused program's stack.
    fn path_keys(&self) -> Vec<String> {
        let Some(project) = &self.project else {
            return Vec::new();
        };
        self.on_stack
            .iter()
            .map(|&id| super::super::plan::function_key(project, id))
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_scene(
        &self,
        scene: &Scene,
        view: View,
        opacity: f32,
        live: bool,
        lit: &Lit,
        now: Instant,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let path = &lit.path;
        // The arrows: Plan's behind its boxes, Site's over them.
        let mut lines = Vec::new();
        let mut labels = Vec::new();
        // Flow labels go where they cover no region, no region's name and
        // no other label.
        let mut avoid = obstacles(scene, |rect| {
            let at = view.point((rect.x, rect.y));
            Rect {
                x: f32::from(at.x),
                y: f32::from(at.y),
                width: rect.width * view.camera.zoom,
                height: rect.height * view.camera.zoom,
            }
        });
        for (ix, arrow) in scene.arrows.iter().enumerate() {
            let (from, to) = (view.point(arrow.from), view.point(arrow.to));
            let joined = |key: &str| {
                lit.hovered.as_deref() == Some(key)
                    || lit
                        .selected
                        .iter()
                        .any(|s| s.strip_prefix("site:").unwrap_or(s) == key)
            };
            let on_path = path
                .windows(2)
                .any(|pair| pair[1] == arrow.keys.0 && pair[0] == arrow.keys.1);
            let (fired, pulse) = self.firing.of_call(&arrow.keys.0, &arrow.keys.1, now);
            let mut colour = theme
                .foreground
                .opacity(if arrow.across { 0.5 } else { 0.32 });
            let mut width = match &arrow.calls {
                Some(calls) => (1. + (calls.count as f32).sqrt() * 0.9).clamp(1.2, 7.),
                None => (2. * view.camera.zoom).max(1.),
            };
            // A box selected at Site: the arrows carrying its calls lit, the
            // rest fainter.
            match arrow.lit {
                Some(true) => {
                    colour = theme.foreground.opacity(0.85);
                    width *= 1.4;
                }
                Some(false) => colour = theme.foreground.opacity(0.12),
                None if joined(&arrow.keys.0) || joined(&arrow.keys.1) => {
                    colour = theme.foreground.opacity(0.85);
                    width *= 1.4;
                }
                None => {}
            }
            if on_path {
                colour = theme.blue;
                width *= 1.6;
            }
            if fired > 0. {
                colour = blend(colour, theme.yellow, fired);
                width *= 1. + fired;
            }
            if arrow.link {
                // Domains' links: steady, their flows labelled where shown;
                // the selected domain's stronger, in its colour.
                width = if arrow.picked || joined(&arrow.keys.0) || joined(&arrow.keys.1) {
                    3.
                } else {
                    2.
                };
                if arrow.picked {
                    colour = match arrow.tint {
                        Some(tint) => domain::colour(tint, theme.mode.is_dark()),
                        None => theme.foreground.opacity(0.85),
                    };
                }
                if live && !arrow.labels.is_empty() {
                    let ends = (
                        (f32::from(from.x), f32::from(from.y)),
                        (f32::from(to.x), f32::from(to.y)),
                    );
                    let size = flow_label_size(&arrow.labels, arrow.title.as_deref());
                    let spot = self.sticky_spot(arrow, ends, size, &avoid, view);
                    let placed = Rect {
                        x: spot.0,
                        y: spot.1,
                        width: size.0,
                        height: size.1,
                    };
                    avoid.push(placed);
                    // Pushed out of the way: a line back to its arrow, in the
                    // arrow's colour and ending in a dot on it when it's the
                    // selected domain's, else faint.
                    if let Some((on, edge)) = leader(ends.0, ends.1, placed) {
                        let (shade, width, dot) = if arrow.picked {
                            (colour.opacity(0.7 * arrow.alpha), 1.5, Some((1., colour)))
                        } else {
                            let a = if arrow.faint { 0.08 } else { 0.22 };
                            (theme.foreground.opacity(a * arrow.alpha), 1., None)
                        };
                        lines.push(Line {
                            from: point(px(edge.0), px(edge.1)),
                            to: point(px(on.0), px(on.1)),
                            colour: shade,
                            width,
                            dashed: !arrow.picked,
                            across: false,
                            zoom: 1.,
                            pulse: dot,
                            heads: (false, false),
                        });
                    }
                    labels.push(self.render_flow_label(ix, arrow, spot, size, from, to, cx));
                }
            }
            lines.push(Line {
                from,
                to,
                colour: colour.opacity(colour.a * arrow.alpha),
                width,
                dashed: arrow.dashed,
                across: arrow.across,
                zoom: view.camera.zoom,
                pulse: pulse.map(|t| (t, theme.yellow)),
                heads: arrow.heads,
            });
        }
        let across = scene.arrows.first().is_some_and(|a| a.across);
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
        .size_full();
        let frames: Vec<AnyElement> = scene
            .frames
            .iter()
            .map(|frame| self.render_frame(frame, view, lit, cx))
            .collect();
        let cards: Vec<AnyElement> = scene
            .cards
            .iter()
            .map(|card| self.render_card(card, view, live, lit, now, cx))
            .collect();
        let regions: Vec<AnyElement> = scene
            .regions
            .iter()
            .flat_map(|region| self.render_region(region, view, lit, cx))
            .collect();
        let _ = window;
        let paint = paint_holder(paint.into_any_element());
        let (under, over) = if across {
            (Some(paint), None)
        } else {
            (None, Some(paint))
        };
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .opacity(opacity)
            .children(regions)
            .children(frames)
            .children(under)
            .children(cards)
            .children(over)
            .children(labels)
            .into_any_element()
    }

    fn render_frame(
        &self,
        frame: &FrameBox,
        view: View,
        lit: &Lit,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let at = view.point((frame.rect.x, frame.rect.y));
        let (width, height) = (
            view.scaled(frame.rect.width),
            view.scaled(frame.rect.height),
        );
        let selected = lit
            .selected
            .iter()
            .any(|key| key == &format!("site:{}", frame.key));
        let accent = match frame.tint {
            Some(tint) => domain::colour(tint, theme.mode.is_dark()),
            None => look_colour(frame.look, cx),
        };
        let fixed = frame.look.fixed_text();
        // Site's titles fit their title bar, at most the size cards use.
        let bar = frame.header * view.camera.zoom;
        let title_size = if fixed {
            px((bar * 0.55).min(12.))
        } else {
            view.scaled(11.)
        };
        let words = !fixed || bar >= 13.;
        let package = frame.look == Look::Package;
        div()
            .absolute()
            .left(at.x)
            .top(at.y)
            .w(width)
            .h(height)
            .rounded(if fixed { px(8.) } else { view.scaled(10.) })
            .border_1()
            .when(package, |this| this.border_2())
            .border_color(if selected {
                theme.primary
            } else {
                accent.opacity(if package { 0.45 } else { 0.25 })
            })
            .bg(accent.opacity(0.035))
            .opacity(frame.alpha)
            .overflow_hidden()
            .when(words, |this| {
                this.child(
                    h_flex()
                        .h(if fixed { px(bar) } else { view.scaled(24.) })
                        .px(if fixed { px(10.) } else { view.scaled(10.) })
                        .gap_2()
                        .min_w_0()
                        .items_center()
                        .text_size(title_size)
                        .child(
                            div()
                                .flex_none()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.foreground.opacity(0.8))
                                .child(frame.title.clone()),
                        )
                        .when(!frame.note.is_empty() && fixed, |this| {
                            this.child(
                                div()
                                    .flex_none()
                                    .text_size(px((bar * 0.48).min(10.5)))
                                    .text_color(theme.muted_foreground)
                                    .child(frame.note.clone()),
                            )
                        }),
                )
            })
            .into_any_element()
    }

    fn render_card(
        &self,
        card: &Card,
        view: View,
        live: bool,
        lit: &Lit,
        now: Instant,
        cx: &Context<Self>,
    ) -> AnyElement {
        if card.look.fixed_text() {
            self.render_site_card(card, view, lit, cx)
        } else {
            self.render_plan_card(card, view, live, lit, now, cx)
        }
    }

    /// A Site box: its name, what it's for, how much it holds, at one
    /// size whatever the zoom, as much as fits.
    fn render_site_card(
        &self,
        card: &Card,
        view: View,
        lit: &Lit,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let at = view.point((card.rect.x, card.rect.y));
        let (width, height) = (
            card.rect.width * view.camera.zoom,
            card.rect.height * view.camera.zoom,
        );
        let accent = match card.tint {
            Some(tint) => domain::colour(tint, theme.mode.is_dark()),
            None => look_colour(card.look, cx),
        };
        let selected = lit
            .selected
            .iter()
            .any(|key| key == &format!("site:{}", card.key) || key == &card.key);
        let hovered = lit.hovered.as_deref() == Some(card.key.as_str());
        let words = width >= 46. && height >= 16.;
        let unread = card.look == Look::Unread;
        // As many lines as fit: the title (two at most), what it's for or
        // holds, then how much.
        let lines = ((height - 10.) / 14.).floor().max(1.) as usize;
        let title_lines = if lines >= 3 { 2 } else { 1 };
        let detail_lines = lines.saturating_sub(title_lines + 1);
        div()
            .absolute()
            .left(at.x)
            .top(at.y)
            .w(px(width))
            .h(px(height))
            .overflow_hidden()
            .rounded(px((height / 6.).clamp(2., 7.)))
            .border_1()
            .when(unread, |this| this.border_dashed())
            .border_color(if selected {
                theme.primary
            } else if hovered {
                accent.opacity(0.8)
            } else {
                accent.opacity(0.4)
            })
            .when(selected, |this| this.border_2().shadow_md())
            .bg(blend(
                theme.background,
                accent,
                match (unread, card.tint.is_some()) {
                    (true, _) => 0.04,
                    (false, true) => 0.18,
                    (false, false) => 0.1,
                },
            ))
            .opacity(card.alpha)
            .when(words, |this| {
                this.child(
                    v_flex()
                        .px(px(8.))
                        .py(px(if lines >= 2 { 5. } else { 1. }))
                        .gap(px(1.))
                        .child(
                            div()
                                .line_height(px(14.))
                                .line_clamp(title_lines)
                                .text_size(px(12.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(if unread {
                                    theme.muted_foreground
                                } else {
                                    theme.foreground
                                })
                                .child(card.title.clone()),
                        )
                        .when(detail_lines > 0 && !card.detail.is_empty(), |this| {
                            this.child(
                                div()
                                    .line_height(px(13.))
                                    .line_clamp(detail_lines)
                                    .text_size(px(10.5))
                                    .text_color(theme.foreground.opacity(0.7))
                                    .child(card.detail.clone()),
                            )
                        })
                        .when(lines >= 2, |this| {
                            this.child(
                                div()
                                    .truncate()
                                    .text_size(px(10.))
                                    .text_color(theme.muted_foreground)
                                    .child(card.note.clone()),
                            )
                        }),
                )
            })
            .into_any_element()
    }

    /// A Plan box: the function's name in its header, its signature
    /// below, the hubs it calls as badges, and buttons opening the hops
    /// not drawn.
    fn render_plan_card(
        &self,
        card: &Card,
        view: View,
        live: bool,
        lit: &Lit,
        now: Instant,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let ix = card.ix;
        let at = view.point((card.rect.x, card.rect.y));
        let scaled = |value: f32| view.scaled(value);
        let accent = look_colour(card.look, cx);
        let selected = lit.selected.contains(&card.key);
        let on_path = lit.path.iter().position(|key| key == &card.key);
        let fired = self.firing.of_box(&card.key, now);
        let dimmed = lit
            .hovered
            .as_ref()
            .is_some_and(|hovered| hovered != &card.key && !self.joined(hovered, &card.key));
        let gone = card.look == Look::Gone;
        let expander = |id: &'static str, label: String, what: Expand, tip: &'static str| {
            div()
                .id((id, ix))
                .debug_selector(move || format!("{id}-{ix}"))
                .flex_none()
                .px(scaled(4.))
                .rounded(scaled(4.))
                .text_size(scaled(10.5))
                .text_color(gpui_kit::white().opacity(0.8))
                .hover(|style| style.bg(gpui_kit::white().opacity(0.18)))
                .child(label)
                .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.expand(ix, what, cx);
                }))
        };
        let [callers, callees, implementations] = card.hidden;
        let buttons = live.then(|| {
            (
                (callers > 0).then(|| {
                    expander(
                        "map-callers",
                        format!("◀ {callers}"),
                        Expand::Callers,
                        "Show its callers",
                    )
                }),
                (callees > 0).then(|| {
                    expander(
                        "map-callees",
                        format!("{callees} ▶"),
                        Expand::Callees,
                        "Show what it calls",
                    )
                }),
                (implementations > 0).then(|| {
                    expander(
                        "map-implementations",
                        format!("{implementations} impl ▶"),
                        Expand::Implementations,
                        "Show its implementations",
                    )
                }),
            )
        });
        let (before, after, impls) = match buttons {
            Some((a, b, c)) => (a, b, c),
            None => (None, None, None),
        };
        let detail = (!card.detail.is_empty()).then(|| {
            div()
                .h(scaled(ROW_HEIGHT))
                .px(scaled(10.))
                .truncate()
                .when(
                    matches!(card.look, Look::Function | Look::Abstract),
                    |this| {
                        this.font_family(theme.mono_font_family.clone())
                            .text_size(scaled(10.5))
                    },
                )
                .text_color(theme.muted_foreground)
                .child(card.detail.clone())
        });
        let badges = (!card.badges.is_empty()).then(|| {
            h_flex()
                .h(scaled(ROW_HEIGHT))
                .px(scaled(10.))
                .gap(scaled(4.))
                .overflow_hidden()
                .children(card.badges.iter().take(4).map(|badge| {
                    div()
                        .flex_none()
                        .px(scaled(4.))
                        .rounded(scaled(4.))
                        .bg(theme.foreground.opacity(0.08))
                        .text_color(theme.muted_foreground)
                        .text_size(scaled(10.))
                        .child(badge.clone())
                }))
        });
        let unknown = card.unknown_target.then(|| {
            div()
                .flex_none()
                .px(scaled(4.))
                .rounded(scaled(4.))
                .border_1()
                .border_dashed()
                .border_color(gpui_kit::white().opacity(0.6))
                .text_size(scaled(10.))
                .child("?")
                .id(("map-unknown", ix))
                .tooltip(|window, cx| {
                    Tooltip::new(
                        "It calls a function it's handed; which one, nothing here can know",
                    )
                    .build(window, cx)
                })
        });
        let checked = card.checked.then(|| {
            div()
                .id(("map-checked", ix))
                .flex_none()
                .text_size(scaled(10.))
                .child("✓")
                .tooltip(|window, cx| {
                    Tooltip::new("Calls checked with the language server").build(window, cx)
                })
        });
        let note = (!card.note.is_empty()).then(|| {
            div()
                .h(scaled(ROW_HEIGHT))
                .px(scaled(10.))
                .truncate()
                .italic()
                .text_size(scaled(10.5))
                .text_color(theme.muted_foreground)
                .child(card.note.clone())
        });
        let body = (detail.is_some() || badges.is_some() || note.is_some()).then(|| {
            v_flex()
                .py(scaled(PADDING))
                .children(detail)
                .children(badges)
                .children(note)
        });
        let border = if selected {
            theme.primary
        } else if on_path.is_some() {
            theme.blue
        } else if card.focus {
            accent
        } else {
            theme.border
        };
        div()
            .id(("map-box", ix))
            .debug_selector(move || format!("map-box-{ix}"))
            .absolute()
            .left(at.x)
            .top(at.y)
            .w(scaled(card.rect.width))
            .h(scaled(card.rect.height))
            .overflow_hidden()
            .rounded(scaled(7.))
            .border_1()
            .when(
                selected || card.focus || on_path.is_some() || fired > 0.,
                |this| this.border_2(),
            )
            .border_color(if fired > 0. {
                blend(border, theme.yellow, fired)
            } else {
                border
            })
            .when(selected || on_path == Some(0), |this| this.shadow_lg())
            .when(!selected, |this| this.shadow_sm())
            .bg(blend(
                match card.tint {
                    Some(tint) => blend(
                        theme::card(cx),
                        domain::colour(tint, theme.mode.is_dark()),
                        0.14,
                    ),
                    None => theme::card(cx),
                },
                theme.yellow,
                fired * 0.25,
            ))
            .opacity(card.alpha * if gone { 0.7 } else { 1. } * if dimmed { 0.5 } else { 1. })
            .text_color(theme.foreground)
            .text_size(scaled(11.5))
            .line_height(scaled(ROW_HEIGHT))
            .child(
                h_flex()
                    .h(scaled(HEADER_HEIGHT))
                    .gap(scaled(4.))
                    .px(scaled(8.))
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(accent.opacity(if gone { 0.4 } else { 0.95 }), 0.),
                        linear_color_stop(
                            Hsla {
                                l: accent.l * 0.6,
                                ..accent
                            }
                            .opacity(if gone { 0.4 } else { 0.92 }),
                            1.,
                        ),
                    ))
                    .font_weight(FontWeight::BOLD)
                    .text_color(gpui_kit::white())
                    .children(before)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(card.title.clone()),
                    )
                    .children(checked)
                    .children(unknown)
                    .children(impls)
                    .children(after),
            )
            .children(body)
            .into_any_element()
    }

    /// Whether Plan boxes `a` and `b` are joined by a call.
    fn joined(&self, a: &str, b: &str) -> bool {
        self.drawn.borrow().arrows.iter().any(|arrow| {
            (arrow.keys.0 == a && arrow.keys.1 == b) || (arrow.keys.0 == b && arrow.keys.1 == a)
        })
    }

    fn render_marquee(&self, cx: &App) -> Option<AnyElement> {
        let Some(super::input::Drag::Marquee { from, to, .. }) = &self.drag else {
            return None;
        };
        let theme = cx.theme();
        let origin = self.bounds.get().origin;
        let (a, b) = (*from - origin, *to - origin);
        Some(
            div()
                .absolute()
                .left(a.x.min(b.x))
                .top(a.y.min(b.y))
                .w((a.x - b.x).abs())
                .h((a.y - b.y).abs())
                .border_1()
                .border_color(theme.foreground.opacity(0.5))
                .bg(theme.foreground.opacity(0.06))
                .into_any_element(),
        )
    }

    /// What the pointer rests on: a function's code at Plan, the calls
    /// behind an arrow at Site.
    fn render_hover(&self, cx: &App) -> Option<AnyElement> {
        let theme = cx.theme();
        let (at, lines, mono) = match self.hover.as_ref()? {
            Hover::Card(key, since) if since.elapsed().as_millis() >= input::PEEK_MS => {
                let lines = self.peek(key)?;
                let ix = self.plan.index_of(key)?;
                let rect = self.placed.nodes[ix];
                (self.to_canvas((rect.x, rect.bottom() + 6.)), lines, true)
            }
            Hover::Arrow(ix, position) if self.level == Level::Domain => {
                let arrow = self.drawn.borrow().arrows.get(*ix)?.clone();
                let name = |key: &str| key.strip_prefix("domain:").unwrap_or(key).to_string();
                let (a, b) = (name(&arrow.keys.0), name(&arrow.keys.1));
                let lines = arrow
                    .labels
                    .iter()
                    .map(|(forward, label)| {
                        if *forward {
                            format!("{a} → {b}: {label}")
                        } else {
                            format!("{b} → {a}: {label}")
                        }
                    })
                    .collect();
                (
                    *position - self.bounds.get().origin + point(px(12.), px(12.)),
                    lines,
                    false,
                )
            }
            Hover::Arrow(ix, position) => {
                let arrow = self.drawn.borrow().arrows.get(*ix)?.clone();
                let calls = arrow.calls?;
                let mut lines = vec![format!(
                    "{} call{}",
                    calls.count,
                    if calls.count == 1 { "" } else { "s" }
                )];
                lines.extend(calls.examples.iter().cloned());
                if calls.count > calls.examples.len() {
                    lines.push("…".into());
                }
                (
                    *position - self.bounds.get().origin + point(px(12.), px(12.)),
                    lines,
                    false,
                )
            }
            _ => return None,
        };
        Some(
            v_flex()
                .id("map-hover")
                .debug_selector(|| "map-hover".into())
                .absolute()
                .left(at.x)
                .top(at.y)
                .max_w(px(520.))
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .shadow_lg()
                .text_size(px(11.))
                .when(mono, |this| {
                    this.font_family(theme.mono_font_family.clone())
                })
                .text_color(theme.popover_foreground)
                .children(
                    lines
                        .into_iter()
                        .map(|line| div().whitespace_nowrap().truncate().child(line)),
                )
                .into_any_element(),
        )
    }

    fn render_hint(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let notice = self
            .notice
            .as_ref()
            .filter(|(_, at)| at.elapsed().as_secs() < 4)
            .map(|(text, _)| text.clone());
        let drawing = match self.tool() {
            super::sketch::Tool::Pointer => None,
            super::sketch::Tool::Arrow => {
                Some("Drag from a box to another, or off a box to a new one · Esc for the pointer")
            }
            super::sketch::Tool::Note => Some(
                "Click a box to note on it, or anywhere for a loose note · Esc for the pointer",
            ),
            _ => Some("Click or drag to draw · Esc for the pointer"),
        };
        let text = notice.or(drawing.map(String::from)).unwrap_or_else(|| {
            match self.level {
                Level::Domain if self.drafts() => {
                    "Jig's own grouping, from folders and calls · zoom into a group to open it onto its code · ⌘↓ or Enter enters"
                }
                Level::Domain => {
                    "Zoom into a domain to open it onto its code · drop a domain onto another to merge · double-click a name to rename · ⌘Z undoes"
                }
                Level::Site => {
                    "Zoom in: each domain opens onto its components, folders and files, a file into its Plan · ⌘↓, Enter or double-click enters · ⌘↑ or zoom out to Domain"
                }
                Level::Plan(_) => {
                    "Zoom into a function for its Plan, into this one for its code · double-click or Enter enters · ◀ ▶ open a hop · ⌘↑ out to Site · Esc back to the code"
                }
            }
            .to_string()
        });
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

use super::input;

/// The arrows' painting, over or under the boxes as the level draws them.
fn paint_holder(paint: AnyElement) -> AnyElement {
    div()
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .child(paint)
        .into_any_element()
}

/// The colour each kind of box is drawn in.
pub(super) fn look_colour(look: Look, cx: &App) -> Hsla {
    let theme = cx.theme();
    match look {
        Look::Function => theme.blue,
        Look::Abstract => theme.cyan,
        Look::External | Look::More | Look::Gone => theme.muted_foreground,
        Look::Folder | Look::Owner => theme.foreground,
        Look::Package => theme.magenta,
        Look::File => theme.blue,
        Look::Unread => theme.muted_foreground,
        Look::Type => theme.green,
        Look::Source => theme.cyan,
        Look::UnreadSource => theme.muted_foreground,
    }
}

/// `a` turned `t` of the way to `b`.
pub(super) fn blend(a: Hsla, b: Hsla, t: f32) -> Hsla {
    let (a, b, t) = (Rgba::from(a), Rgba::from(b), t.clamp(0., 1.));
    Hsla::from(Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    })
}

/// An arrow as painted, in canvas points.
pub(super) struct Line {
    pub(super) from: Point<Pixels>,
    pub(super) to: Point<Pixels>,
    pub(super) colour: Hsla,
    pub(super) width: f32,
    pub(super) dashed: bool,
    pub(super) across: bool,
    pub(super) zoom: f32,
    /// How far down it a pulse is, and its colour.
    pub(super) pulse: Option<(f32, Hsla)>,
    /// Heads where it leaves and where it arrives.
    pub(super) heads: (bool, bool),
}

impl Line {
    pub(super) fn paint(self, origin: Point<Pixels>, window: &mut Window) {
        let (from, to) = (origin + self.from, origin + self.to);
        let mut path = PathBuilder::stroke(px(self.width));
        if self.dashed {
            let dash = px((6. * self.zoom).max(3.));
            path = path.dash_array(&[dash, dash]);
        }
        path.move_to(from);
        let (a, b) = self.controls(from, to);
        path.cubic_bezier_to(to, a, b);
        if let Ok(built) = path.build() {
            window.paint_path(built, self.colour);
        }
        let middle = bow_point(from, to, 0.5);
        if self.heads.1 {
            self.paint_head(middle, to, window);
        }
        if self.heads.0 {
            self.paint_head(middle, from, window);
        }
        if let Some((t, colour)) = self.pulse {
            let at = cubic(from, a, b, to, t);
            let r = px(self.width * 1.6 + 2.);
            let mut dot = PathBuilder::fill();
            dot.move_to(point(at.x + r, at.y));
            dot.arc_to(point(r, r), px(0.), false, true, point(at.x - r, at.y));
            dot.arc_to(point(r, r), px(0.), false, true, point(at.x + r, at.y));
            if let Ok(built) = dot.build() {
                window.paint_path(built, colour);
            }
        }
    }

    /// The curve's control points: across, out of one box's right into
    /// the next one's left; else bowed to one side, so the arrows either
    /// way between two boxes don't lie on each other.
    fn controls(&self, from: Point<Pixels>, to: Point<Pixels>) -> (Point<Pixels>, Point<Pixels>) {
        if self.across {
            let reach = px((f32::from(to.x - from.x).abs() / 2.).max(40. * self.zoom));
            return (point(from.x + reach, from.y), point(to.x - reach, to.y));
        }
        let middle = bow_point(from, to, 0.5);
        (middle, middle)
    }

    /// A head at `to`, pointing along the line from `middle`.
    fn paint_head(&self, middle: Point<Pixels>, to: Point<Pixels>, window: &mut Window) {
        let (dx, dy) = (f32::from(to.x - middle.x), f32::from(to.y - middle.y));
        let length = dx.hypot(dy).max(0.001);
        let (ux, uy) = (dx / length, dy / length);
        let size = 5. + self.width * 1.5;
        let back = point(to.x - px(ux * size), to.y - px(uy * size));
        let side = point(px(-uy * size * 0.55), px(ux * size * 0.55));
        let mut head = PathBuilder::fill();
        head.move_to(to);
        head.line_to(back + side);
        head.line_to(back - side);
        head.close();
        if let Ok(built) = head.build() {
            window.paint_path(built, self.colour);
        }
    }
}

/// What flow labels keep clear of, on screen (`screen` turns a drawing
/// rect into one): each region, and its name above it.
pub(super) fn obstacles(scene: &Scene, screen: impl Fn(Rect) -> Rect) -> Vec<Rect> {
    let mut avoid = Vec::new();
    for region in &scene.regions {
        let rect = screen(region.rect);
        let quiet = !matches!(
            region.kind,
            super::super::domains::level::RegionKind::Domain(_)
        );
        let zoom = rect.width / region.rect.width.max(0.0001);
        let name = super::domain_draw::name_label(
            &region.name,
            quiet,
            super::domain_draw::name_room(rect.width, zoom),
        );
        avoid.push(rect);
        avoid.push(Rect {
            x: rect.x,
            y: rect.y - name.height,
            width: name.width,
            height: name.height,
        });
    }
    avoid
}

/// The widest a flow label gets, on screen.
pub(super) const FLOW_LABEL_WIDTH: f32 = 180.;

/// How big a flow label is on screen, roughly: its lines at 10.5 points,
/// wrapping at `FLOW_LABEL_WIDTH`.
pub(super) fn flow_label_size(labels: &[(bool, String)], title: Option<&str>) -> (f32, f32) {
    let per_char = 5.9;
    let room = FLOW_LABEL_WIDTH - 14.;
    let lines = labels
        .iter()
        .map(|(_, label)| label.chars().count() + 2)
        .chain(title.map(|title| title.chars().count()));
    let widest = lines
        .clone()
        .map(|chars| chars as f32 * per_char)
        .fold(0., f32::max);
    let count: f32 = lines
        .map(|chars| ((chars as f32 * per_char) / room).ceil().max(1.))
        .sum();
    ((widest + 14.).min(FLOW_LABEL_WIDTH), count * 14. + 6.)
}

/// Where a label `size` big goes for the bowed arrow from `from` to `to`,
/// on screen: its top left. On the arrow when that's clear, else the
/// clear spot nearest it, looking further out step by step, inside
/// `within` and covering none of `avoid`; if nothing near is clear, the
/// spot covering least.
pub(super) fn label_spot(
    from: (f32, f32),
    to: (f32, f32),
    size: (f32, f32),
    avoid: &[Rect],
    within: Rect,
) -> (f32, f32) {
    let curve: Vec<(f32, f32)> = (0..=16)
        .map(|step| {
            let at = bow_point(
                point(px(from.0), px(from.1)),
                point(px(to.0), px(to.1)),
                step as f32 / 16.,
            );
            (f32::from(at.x), f32::from(at.y))
        })
        .collect();
    let middle = curve[8];
    let place = |centre: (f32, f32)| Rect {
        x: centre.0 - size.0 / 2.,
        y: centre.1 - size.1 / 2.,
        width: size.0,
        height: size.1,
    };
    let covered = |rect: &Rect| -> f32 {
        let outside = rect.width * rect.height - overlap(rect, &within);
        avoid.iter().map(|other| overlap(rect, other)).sum::<f32>() + outside
    };
    // How far a spot is from the arrow, and from its middle.
    let away = |rect: &Rect| {
        let nearest = curve
            .iter()
            .map(|p| {
                let q = (
                    p.0.clamp(rect.x, rect.right()),
                    p.1.clamp(rect.y, rect.bottom()),
                );
                (p.0 - q.0).hypot(p.1 - q.1)
            })
            .fold(f32::MAX, f32::min);
        let centre = rect.centre();
        nearest + (centre.0 - middle.0).hypot(centre.1 - middle.1) * 0.15
    };
    let mut best: Option<(Rect, f32)> = None;
    let mut least: Option<(Rect, f32)> = None;
    const STEP: f32 = 18.;
    for &on in curve.iter().skip(4).step_by(2).take(5) {
        for ring in 0..=18i32 {
            for dx in -ring..=ring {
                for dy in -ring..=ring {
                    if dx.abs().max(dy.abs()) != ring {
                        continue;
                    }
                    let rect = place((on.0 + dx as f32 * STEP, on.1 + dy as f32 * STEP));
                    let cover = covered(&rect);
                    if cover <= 0. {
                        let distance = away(&rect);
                        if best.is_none_or(|(_, d)| distance < d) {
                            best = Some((rect, distance));
                        }
                    } else if least.is_none_or(|(_, c)| cover < c) {
                        least = Some((rect, cover));
                    }
                }
            }
            // A clear spot this close won't be beaten by one further out.
            if best.is_some_and(|(_, d)| d <= ring as f32 * STEP) {
                break;
            }
        }
    }
    best.or(least).map_or(
        (middle.0 - size.0 / 2., middle.1 - size.1 / 2.),
        |(rect, _)| (rect.x, rect.y),
    )
}

/// The line from the bowed arrow from `from` to `to` to its label at
/// `label`, when the label sits apart from it.
pub(super) fn leader(
    from: (f32, f32),
    to: (f32, f32),
    label: Rect,
) -> Option<((f32, f32), (f32, f32))> {
    let near_label = |at: (f32, f32)| {
        (
            at.0.clamp(label.x, label.right()),
            at.1.clamp(label.y, label.bottom()),
        )
    };
    let (on, edge) = (0..=20)
        .map(|step| {
            let at = bow_point(
                point(px(from.0), px(from.1)),
                point(px(to.0), px(to.1)),
                step as f32 / 20.,
            );
            let at = (f32::from(at.x), f32::from(at.y));
            (at, near_label(at))
        })
        .min_by(|a, b| {
            let d = |(p, q): &((f32, f32), (f32, f32))| (p.0 - q.0).hypot(p.1 - q.1);
            d(a).total_cmp(&d(b))
        })?;
    ((on.0 - edge.0).hypot(on.1 - edge.1) > 14.).then_some((on, edge))
}

fn overlap(a: &Rect, b: &Rect) -> f32 {
    let w = (a.right().min(b.right()) - a.x.max(b.x)).max(0.);
    let h = (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.);
    w * h
}

/// The point `t` along a bowed arrow from `from` to `to`: the control
/// point sits off the middle, to the arrow's left.
pub(super) fn bow_point(from: Point<Pixels>, to: Point<Pixels>, t: f32) -> Point<Pixels> {
    let (dx, dy) = (f32::from(to.x - from.x), f32::from(to.y - from.y));
    let bend = 0.12;
    let control = point(
        from.x + px(dx / 2. + dy * bend),
        from.y + px(dy / 2. - dx * bend),
    );
    if t == 0.5 {
        return control;
    }
    let u = 1. - t;
    point(
        from.x * (u * u) + control.x * (2. * u * t) + to.x * (t * t),
        from.y * (u * u) + control.y * (2. * u * t) + to.y * (t * t),
    )
}

fn cubic(
    p0: Point<Pixels>,
    p1: Point<Pixels>,
    p2: Point<Pixels>,
    p3: Point<Pixels>,
    t: f32,
) -> Point<Pixels> {
    let u = 1. - t;
    let (a, b, c, d) = (u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t);
    point(
        p0.x * a + p1.x * b + p2.x * c + p3.x * d,
        p0.y * a + p1.y * b + p2.y * c + p3.y * d,
    )
}

fn paint_grid(
    bounds: Bounds<Pixels>,
    offset: Point<Pixels>,
    zoom: f32,
    colour: Hsla,
    window: &mut Window,
) {
    let step = GRID * zoom;
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

/// The Map's colours over the theme's.
pub(super) mod theme {
    use gpui_kit::component::ActiveTheme as _;
    use gpui_kit::*;

    /// Behind everything: the editor's surface, a shade off.
    pub fn canvas(cx: &App) -> Hsla {
        let surface = jig_commands::surface::editor_surface(cx);
        let l = if surface.l > 0.5 {
            surface.l - 0.025
        } else {
            surface.l + 0.02
        };
        Hsla { l, ..surface }
    }

    /// A Plan box's body.
    pub fn card(cx: &App) -> Hsla {
        cx.theme().background
    }
}
