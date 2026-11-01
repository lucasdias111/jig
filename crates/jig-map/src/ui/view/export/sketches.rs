//! The user's sketches in an exported image.

use super::*;

/// The user's sketches, as the canvas draws them, in SVG: neutral ink,
/// dashed while they're ideas, solid once real.
pub(super) fn draw_sketches(out: &mut String, picture: &SketchPicture, palette: &Palette) {
    let ink = hex(palette.foreground);
    let dash = |real: bool| {
        if real {
            ""
        } else {
            r#" stroke-dasharray="7 5""#
        }
    };
    for frame in &picture.frames {
        let r = frame.rect;
        let _ = write!(
            out,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="14" fill="{ink}" fill-opacity="0.025" stroke="{ink}" stroke-opacity="0.55" stroke-width="2" stroke-dasharray="8 6"/>"#,
            r.x, r.y, r.width, r.height
        );
        text(
            out,
            r.x + 6.,
            r.y - 7.,
            13.,
            palette.foreground,
            true,
            &frame.label,
        );
    }
    for (from, to) in &picture.ties {
        let _ = write!(
            out,
            r#"<line x1="{}" y1="{}" x2="{}" y2="{}" stroke="{ink}" stroke-opacity="0.45" stroke-dasharray="4 4"/>"#,
            from.0, from.1, to.0, to.1
        );
    }
    for arrow in &picture.arrows {
        let middle = bow(arrow.from, arrow.to);
        let _ = write!(
            out,
            r#"<path d="M{} {} Q{} {} {} {}" fill="none" stroke="{ink}" stroke-opacity="0.8" stroke-width="1.8"{}/>"#,
            arrow.from.0,
            arrow.from.1,
            middle.0,
            middle.1,
            arrow.to.0,
            arrow.to.1,
            dash(arrow.real),
        );
        head(out, middle, arrow.to, 1.8, palette.foreground);
        if !arrow.label.is_empty() {
            let at = bow_point(
                point(px(arrow.from.0), px(arrow.from.1)),
                point(px(arrow.to.0), px(arrow.to.1)),
                0.5,
            );
            let (x, y) = (f32::from(at.x), f32::from(at.y));
            let width = (arrow.label.chars().count() as f32 * 6.4 + 12.).min(200.);
            let _ = write!(
                out,
                r#"<rect x="{}" y="{}" width="{width}" height="18" rx="4" fill="{}" fill-opacity="0.92"/>"#,
                x - width / 2.,
                y - 9.,
                hex(palette.card)
            );
            text(
                out,
                x - width / 2. + 6.,
                y + 4.,
                11.,
                palette.foreground,
                false,
                &clip(&arrow.label, width - 12., 11.),
            );
        }
    }
    for b in &picture.boxes {
        let r = b.rect;
        let real = matches!(b.state, BoxState::Real(_));
        let _ = write!(
            out,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="{}" fill="{}" fill-opacity="{}" stroke="{ink}" stroke-opacity="0.8" stroke-width="1.6"{}/>"#,
            r.x,
            r.y,
            r.width,
            r.height,
            (r.height * 0.14).clamp(3., 10.),
            hex(palette.card),
            if real { 1. } else { 0.7 },
            dash(real),
        );
        let font = (r.height * 0.24).min(18.);
        if font < 7. {
            continue;
        }
        let pad = (r.height * 0.12).min(10.);
        let status = match &b.state {
            BoxState::Real(place) => {
                Some((format!("✓ {}", short_place(place)), palette_green(palette)))
            }
            BoxState::Unsure(n) => Some((format!("? matches {n} things"), palette.muted)),
            BoxState::Sketched => None,
        };
        let small = (font * 0.72).max(8.);
        let rows = 1. + status.is_some() as u8 as f32;
        let top = r.y + r.height / 2. - (font + small * (rows - 1.)) / 2. + font * 0.8;
        text(
            out,
            r.x + pad,
            top,
            font,
            palette.foreground,
            true,
            &clip(&b.label, r.width - 2. * pad, font),
        );
        if let Some((words, colour)) = status {
            text(
                out,
                r.x + pad,
                top + small * 1.3,
                small,
                colour,
                false,
                &clip(&words, r.width - 2. * pad, small),
            );
        }
    }
    let (paper, words) = note_colours(palette.dark);
    for note in &picture.notes {
        let r = note.rect;
        let _ = write!(
            out,
            r#"<rect x="{}" y="{}" width="{}" height="{}" rx="3" fill="{}"/>"#,
            r.x,
            r.y,
            r.width,
            r.height,
            hex(paper)
        );
        let font = (r.height * 0.17).min(15.);
        let pad = (r.height * 0.1).min(8.);
        for (n, line) in wrap(&note.text, r.width - 2. * pad, font)
            .iter()
            .take(4)
            .enumerate()
        {
            text(
                out,
                r.x + pad,
                r.y + pad + font * (n as f32 + 1.),
                font,
                words,
                false,
                line,
            );
        }
    }
    for bar in &picture.bars {
        let (real, total) = bar.real;
        let mut line = format!("✎ {}", bar.title);
        if total > 0 {
            line.push_str(&format!("   {real} of {total} real"));
        }
        if let Some((_, Some(task))) = &bar.task {
            line.push_str(&format!(
                "   Task: {} · {}/{}",
                task.title, task.done, task.total
            ));
        }
        let width = line.chars().count() as f32 * 6.8 + 16.;
        let _ = write!(
            out,
            r#"<rect x="{}" y="{}" width="{width}" height="22" rx="6" fill="{}" fill-opacity="0.94"/>"#,
            bar.at.0,
            bar.at.1 - 28.,
            hex(palette.card)
        );
        text(
            out,
            bar.at.0 + 8.,
            bar.at.1 - 13.,
            12.,
            palette.foreground,
            true,
            &line,
        );
    }
}
