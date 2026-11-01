//! The agent's drawings in an exported image, with their list below.

use super::*;

/// The agent's drawings, as the canvas draws them, in SVG.
pub(super) fn draw_lens(out: &mut String, picture: &Picture, size: (f32, f32), palette: &Palette) {
    let agent = hex(palette.agent);
    let opacity = |pinned: bool| if pinned { 0.55 } else { 1. };
    for group in &picture.groups {
        let r = group.rect;
        let _ = write!(
            out,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="16" fill="{agent}" fill-opacity="0.04" stroke="{agent}" stroke-opacity="{}" stroke-width="2" stroke-dasharray="8 6"/>"#,
            r.x,
            r.y,
            r.width,
            r.height,
            opacity(group.pinned),
        );
        text(
            out,
            r.x + 12.,
            r.y - 4.,
            12.,
            palette.agent,
            true,
            &group.label,
        );
    }
    for path in &picture.paths {
        let middle = bow(path.from, path.to);
        let _ = write!(
            out,
            r#"<path d="M{} {} Q{} {} {} {}" fill="none" stroke="{agent}" stroke-opacity="{}" stroke-width="{}"{}/>"#,
            path.from.0,
            path.from.1,
            middle.0,
            middle.1,
            path.to.0,
            path.to.1,
            opacity(path.pinned),
            path.width,
            if path.dashed {
                r#" stroke-dasharray="8 6""#
            } else {
                ""
            },
        );
        head(out, middle, path.to, path.width, palette.agent);
    }
    let canvas = Rect {
        x: 0.,
        y: 0.,
        width: size.0,
        height: size.1,
    };
    for marked in &picture.boxes {
        let r = marked.rect;
        // Touring, the step's box stands out and the rest fall back.
        let fade = if picture.touring && !marked.toured {
            0.35
        } else {
            1.
        };
        let _ = write!(out, r#"<g opacity="{fade}">"#);
        if !r.overlaps(&canvas) {
            let (x, y, arrow) = super::super::lens_draw::edge_spot(r, size);
            let _ = write!(out, r#"<circle cx="{x}" cy="{y}" r="13" fill="{agent}"/>"#);
            text(out, x - 6., y + 5., 14., palette.background, true, arrow);
            out.push_str("</g>");
            continue;
        }
        let _ = write!(
            out,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="10" fill="{agent}" fill-opacity="{}" stroke="{agent}" stroke-opacity="{}" stroke-width="{}"/>"#,
            r.x - 4.,
            r.y - 4.,
            r.width + 8.,
            r.height + 8.,
            if marked.toured { 0.2 } else { 0.07 },
            opacity(marked.pinned),
            if marked.toured { 4. } else { 2.5 },
        );
        for (n, badge) in marked.badges.iter().enumerate() {
            let width = 12. + badge.chars().count() as f32 * 7.5;
            let from_right = (marked.badges.len() - 1 - n) as f32;
            let (x, y) = (r.right() + 4. - width - from_right * 44., r.y - 14.);
            let _ = write!(
                out,
                r#"<rect x="{x}" y="{y}" width="{width}" height="22" rx="11" fill="{agent}" fill-opacity="{}"/>"#,
                opacity(marked.pinned),
            );
            text(out, x + 6., y + 15.5, 12., palette.card, true, badge);
        }
        let (font, row) = if marked.toured {
            (14., 19.)
        } else {
            (12.5, 17.)
        };
        let width = r.width.max(if marked.toured { 320. } else { 260. });
        // Each line wrapped to the box, as the canvas does.
        let lines: Vec<String> = super::super::lens_draw::capped(&marked.words)
            .iter()
            .flat_map(|line| wrap(line, width - 16., font))
            .collect();
        if !lines.is_empty() {
            let height = 10. + lines.len() as f32 * row;
            // A frame's words above it, clear of the boxes inside.
            let top = if marked.above {
                r.y - 22. - height
            } else {
                r.bottom() + 8.
            };
            let _ = write!(
                out,
                r#"<rect x="{}" y="{}" width="{width}" height="{height}" rx="8" fill="{}" stroke="{agent}" stroke-opacity="0.7" stroke-width="{}"{}/>"#,
                r.x,
                top,
                hex(palette.card),
                if marked.toured { 2.5 } else { 1. },
                if marked.proposed {
                    r#" stroke-dasharray="5 4""#
                } else {
                    ""
                },
            );
            for (n, line) in lines.iter().enumerate() {
                text(
                    out,
                    r.x + 8.,
                    top + 4. + row * (n as f32 + 0.8),
                    font,
                    palette.foreground,
                    marked.toured,
                    &clip(line, width - 16., font),
                );
            }
        }
        out.push_str("</g>");
    }
    for ghost in &picture.ghosts {
        let r = ghost.rect;
        let _ = write!(
            out,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="8" fill="{}" stroke="{agent}" stroke-width="2" stroke-dasharray="6 4"/>"#,
            r.x,
            r.y,
            r.width,
            r.height,
            hex(palette.card),
        );
        text(
            out,
            r.x + 8.,
            r.y + 18.,
            12.5,
            palette.agent,
            true,
            &clip(&ghost.name, r.width - 16., 12.5),
        );
        text(
            out,
            r.x + 8.,
            r.y + 36.,
            11.,
            palette.foreground,
            false,
            &clip(&ghost.purpose, r.width - 16., 11.),
        );
    }
    for tag in &picture.tags {
        let width = (tag.text.chars().count() as f32 * 6.2 + 12.).min(220.);
        let _ = write!(
            out,
            r#"<rect x="{}" y="{}" width="{width}" height="20" rx="5" fill="{}" stroke="{agent}" stroke-dasharray="4 3"/>"#,
            tag.at.0 - width / 2.,
            tag.at.1 - 10.,
            hex(palette.card),
        );
        text(
            out,
            tag.at.0 - width / 2. + 6.,
            tag.at.1 + 4.,
            11.,
            palette.foreground,
            false,
            &clip(&tag.text, width - 12., 11.),
        );
    }
}
