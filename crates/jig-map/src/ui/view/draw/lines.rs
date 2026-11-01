//! Arrows as curves, and placing their labels where nothing else is.

use super::*;

/// An arrow as painted, in canvas points.
pub(in crate::ui::view) struct Line {
    pub(in crate::ui::view) from: Point<Pixels>,
    pub(in crate::ui::view) to: Point<Pixels>,
    pub(in crate::ui::view) colour: Hsla,
    pub(in crate::ui::view) width: f32,
    pub(in crate::ui::view) dashed: bool,
    pub(in crate::ui::view) across: bool,
    pub(in crate::ui::view) zoom: f32,
    /// How far down it a pulse is, and its colour.
    pub(in crate::ui::view) pulse: Option<(f32, Hsla)>,
    /// Heads where it leaves and where it arrives.
    pub(in crate::ui::view) heads: (bool, bool),
}

impl Line {
    pub(in crate::ui::view) fn paint(self, origin: Point<Pixels>, window: &mut Window) {
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
pub(in crate::ui::view) fn obstacles(scene: &Scene, screen: impl Fn(Rect) -> Rect) -> Vec<Rect> {
    let mut avoid = Vec::new();
    for region in &scene.regions {
        let rect = screen(region.rect);
        let quiet = !matches!(
            region.kind,
            crate::domain::domains::level::RegionKind::Domain(_)
        );
        let zoom = rect.width / region.rect.width.max(0.0001);
        let name = super::super::domain_draw::name_label(
            &region.name,
            quiet,
            super::super::domain_draw::name_room(rect.width, zoom),
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
const FLOW_LABEL_WIDTH: f32 = 180.;

/// How big a flow label is on screen, roughly: its lines at 10.5 points,
/// wrapping at `FLOW_LABEL_WIDTH`.
pub(in crate::ui::view) fn flow_label_size(
    labels: &[(bool, String)],
    title: Option<&str>,
) -> (f32, f32) {
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
pub(in crate::ui::view) fn label_spot(
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
pub(in crate::ui::view) fn leader(
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
pub(in crate::ui::view) fn bow_point(
    from: Point<Pixels>,
    to: Point<Pixels>,
    t: f32,
) -> Point<Pixels> {
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
