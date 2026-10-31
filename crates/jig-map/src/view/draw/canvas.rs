//! The canvas: the scene at the camera, its arrows and their labels kept clear of the boxes.

use super::*;

impl MapView {
    pub(in crate::view) fn render_canvas(
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
            .when(self.tool() != super::super::sketch::Tool::Pointer, |this| {
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
            .map(|&id| super::super::super::plan::function_key(project, id))
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
}
