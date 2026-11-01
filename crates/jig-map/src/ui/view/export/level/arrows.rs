//! The level's arrows in SVG, their labels placed clear of the boxes.

use super::*;

/// The arrows, their labels placed clear of the boxes.
pub(super) fn draw_arrows(out: &mut String, scene: &Scene, pic: &Picture, palette: &Palette) {
    let camera = pic.camera;
    let mut avoid = obstacles(scene, |rect| {
        let (x, y, width, height) = pic.rect_of(rect);
        Rect {
            x,
            y,
            width,
            height,
        }
    });
    let size_of_picture = pic.size;
    for arrow in &scene.arrows {
        let (from, to) = (pic.at(arrow.from), pic.at(arrow.to));
        let colour = match (arrow.picked, arrow.tint) {
            (true, Some(tint)) => domain::colour(tint, palette.dark),
            _ => palette.foreground,
        };
        let width = match &arrow.calls {
            Some(calls) => (1. + (calls.count as f32).sqrt() * 0.9).clamp(1.2, 7.),
            None if arrow.picked => 3.,
            None if arrow.link => 2.,
            None => (2. * camera.zoom).max(1.),
        };
        let (c1, c2) = if arrow.across {
            let reach = ((to.0 - from.0).abs() / 2.).max(40. * camera.zoom);
            ((from.0 + reach, from.1), (to.0 - reach, to.1))
        } else {
            let middle = bow(from, to);
            (middle, middle)
        };
        let dash = if arrow.dashed {
            format!(r#" stroke-dasharray="{0} {0}""#, (6. * camera.zoom).max(3.))
        } else {
            String::new()
        };
        let _ = write!(
            out,
            r#"<path d="M{} {} C{} {} {} {} {} {}" fill="none" stroke="{}" stroke-opacity="{}" stroke-width="{width}"{dash}/>"#,
            from.0,
            from.1,
            c1.0,
            c1.1,
            c2.0,
            c2.1,
            to.0,
            to.1,
            hex(colour),
            if arrow.across { 0.5 } else { 0.35 },
        );
        let middle = bow(from, to);
        if arrow.heads.1 {
            head(out, middle, to, width, colour);
        }
        if arrow.heads.0 {
            head(out, middle, from, width, colour);
        }
        if !arrow.labels.is_empty() {
            let (dx, dy) = (to.0 - from.0, to.1 - from.1);
            let lines: Vec<String> = arrow
                .labels
                .iter()
                .map(|(forward, label)| {
                    let glyph = match (arrow.title.is_some(), forward) {
                        (true, true) => "→",
                        (true, false) => "←",
                        (false, _) => glyph(*forward, dx, dy),
                    };
                    format!("{glyph} {label}")
                })
                .collect();
            let size = flow_label_size(&arrow.labels, arrow.title.as_deref());
            let picture = Rect {
                x: 0.,
                y: 0.,
                width: size_of_picture.0,
                height: size_of_picture.1,
            };
            let (x, y) = label_spot(from, to, size, &avoid, picture);
            let placed = Rect {
                x,
                y,
                width: size.0,
                height: size.1,
            };
            avoid.push(placed);
            if let Some((a, b)) = leader(from, to, placed) {
                let _ = write!(
                    out,
                    r#"<path d="M{} {} L{} {}" stroke="{}" stroke-opacity="0.25" stroke-dasharray="3 3"/>"#,
                    a.0,
                    a.1,
                    b.0,
                    b.1,
                    hex(palette.foreground),
                );
            }
            if !arrow.faint {
                let (stroke, opacity) = if arrow.picked {
                    (colour, 0.7)
                } else {
                    (palette.foreground, 0.18)
                };
                let _ = write!(
                    out,
                    r#"<rect x="{x}" y="{y}" width="{}" height="{}" rx="6" fill="{}" stroke="{}" stroke-opacity="{opacity}"/>"#,
                    size.0,
                    size.1,
                    hex(palette.background),
                    hex(stroke),
                );
            }
            let words = if arrow.faint {
                blend(palette.background, palette.foreground, 0.3)
            } else {
                palette.foreground
            };
            let mut line_y = y + 14.;
            if let Some(title) = &arrow.title {
                let shade = match arrow.title_tint {
                    Some(tint) => domain::colour(tint, palette.dark),
                    None => palette.foreground,
                };
                text(out, x + 6., line_y, 10.5, shade, true, title);
                line_y += 14.;
            }
            for line in &lines {
                for part in wrap(line, size.0 - 12., 10.5) {
                    text(out, x + 6., line_y, 10.5, words, false, &part);
                    line_y += 14.;
                }
            }
        }
    }
}
