//! Which shapes a selection holds, and growing a box to its label.

use super::*;

/// Every shape of `sketch`, as selections name them.
pub(in crate::ui::view) fn shapes_of(sketch: &Sketch) -> Vec<ShapeRef> {
    let shape = |kind: ShapeKind, id: String| ShapeRef {
        sketch: sketch.name.clone(),
        kind,
        id,
    };
    sketch
        .boxes
        .iter()
        .map(|b| shape(ShapeKind::Box, b.id.clone()))
        .chain((0..sketch.arrows.len()).map(|ix| shape(ShapeKind::Arrow, ix.to_string())))
        .chain(
            sketch
                .notes
                .iter()
                .map(|n| shape(ShapeKind::Note, n.id.clone())),
        )
        .chain(
            sketch
                .frames
                .iter()
                .map(|f| shape(ShapeKind::Frame, f.id.clone())),
        )
        .collect()
}

/// `rect` grown by `by` on every side (shrunk, for less than 0).
pub(super) fn grow(rect: Rect, by: f32) -> Rect {
    Rect {
        x: rect.x - by,
        y: rect.y - by,
        width: (rect.width + 2. * by).max(0.),
        height: (rect.height + 2. * by).max(0.),
    }
}
