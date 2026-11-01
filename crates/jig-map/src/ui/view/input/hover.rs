//! Resting the pointer: what's under it, the calls an arrow carries, peeking at code; and scrolling.

use super::*;

pub(super) fn same_hover(a: &Option<Hover>, b: &Option<Hover>) -> bool {
    match (a, b) {
        (Some(Hover::Card(a, _)), Some(Hover::Card(b, _))) => a == b,
        (Some(Hover::Arrow(a, _)), Some(Hover::Arrow(b, _))) => a == b,
        (None, None) => true,
        _ => false,
    }
}

/// How far `at` is from the bowed arrow from `from` to `to`, as drawn.
pub(in crate::ui::view) fn distance_to_bow(
    from: (f32, f32),
    to: (f32, f32),
    at: (f32, f32),
) -> f32 {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let bend = 0.12;
    let control = (from.0 + dx / 2. + dy * bend, from.1 + dy / 2. - dx * bend);
    (0..=16)
        .map(|step| {
            let t = step as f32 / 16.;
            let u = 1. - t;
            let x = from.0 * u * u + control.0 * 2. * u * t + to.0 * t * t;
            let y = from.1 * u * u + control.1 * 2. * u * t + to.1 * t * t;
            (x - at.0).hypot(y - at.1)
        })
        .fold(f32::MAX, f32::min)
}

impl MapView {
    /// The pointer moved, no button down: what it's over lights, and a
    /// peek or a list of calls follows if it rests there.
    pub(in crate::ui::view) fn hover_move(
        &mut self,
        event: &MouseMoveEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let position = event.position;
        let card = match self.level {
            Level::Plan(_) => self
                .plan_box_at(position)
                .map(|ix| self.plan.nodes[ix].key.clone()),
            Level::Site | Level::Domain => None,
        };
        let hover = match card {
            Some(key) => match &self.hover {
                Some(Hover::Card(was, since)) if *was == key => Some(Hover::Card(key, *since)),
                _ => {
                    // Shown once it has rested long enough.
                    cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(PEEK_MS as u64 + 20))
                            .await;
                        this.update(cx, |_, cx| cx.notify()).ok();
                    })
                    .detach();
                    Some(Hover::Card(key, Instant::now()))
                }
            },
            None => self.arrow_at(position).map(|ix| Hover::Arrow(ix, position)),
        };
        let changed =
            !matches!((&hover, &self.hover), (None, None)) && !same_hover(&hover, &self.hover);
        self.hover = hover;
        if changed {
            cx.notify();
        }
    }

    /// The Site arrow nearest `position`, if one is near enough.
    pub(super) fn arrow_at(&self, position: Point<Pixels>) -> Option<usize> {
        let at = self.to_drawing(position);
        let reach = ARROW_REACH / self.zoom;
        let drawn = self.drawn.borrow();
        drawn
            .arrows
            .iter()
            .enumerate()
            .filter(|(_, arrow)| !arrow.across)
            .map(|(ix, arrow)| (ix, distance_to_bow(arrow.from, arrow.to, at)))
            .filter(|(_, d)| *d < reach)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(ix, _)| ix)
    }

    /// The first lines of the code of Plan box `key`'s function.
    pub(in crate::ui::view) fn peek(&self, key: &str) -> Option<Vec<String>> {
        let project = self.project.as_ref()?;
        let ix = self.plan.index_of(key)?;
        let Item::Function(id) = self.plan.nodes[ix].item else {
            return None;
        };
        let function = &project.functions[id];
        let text = Folder(&self.root).text(&project.files[function.file].path)?;
        let code = text.get(function.range().clone())?;
        let mut lines: Vec<String> = code
            .lines()
            .take(PEEK_LINES)
            .map(|line| line.replace('\t', "    "))
            .collect();
        if code.lines().count() > PEEK_LINES {
            lines.push("…".into());
        }
        // Lined up as written, the first line's indent dropped.
        let indent = code
            .lines()
            .nth(1)
            .map_or(0, |line| line.len() - line.trim_start().len())
            .saturating_sub(4);
        Some(
            lines
                .into_iter()
                .enumerate()
                .map(|(n, line)| {
                    if n == 0 {
                        line
                    } else {
                        line.chars()
                            .skip(indent.min(line.len() - line.trim_start().len()))
                            .collect()
                    }
                })
                .collect(),
        )
    }

    pub(in crate::ui::view) fn scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let delta = event.delta.pixel_delta(px(16.));
        if event.modifiers.secondary() || event.modifiers.control {
            self.zoom_at(1. - f32::from(delta.y) / 300., event.position, cx);
        } else {
            self.transition = None;
            self.offset += delta;
            self.moved(cx);
            cx.notify();
        }
    }
}
