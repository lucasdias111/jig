//! The level showing, drawn as SVG.

use super::*;
mod arrows;
use arrows::*;

/// The SVG of `scene` seen by `camera` on a picture `size` big.
pub(super) fn draw_svg(
    scene: &Scene,
    camera: Camera,
    size: (f32, f32),
    palette: &Palette,
    cx: &App,
) -> String {
    let pic = Picture { camera, size };
    let mut out = String::new();
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="{FONT}"><rect width="{w}" height="{h}" fill="{bg}"/>"#,
        w = size.0.ceil(),
        h = size.1.ceil(),
        bg = hex(palette.background),
    );
    draw_regions(&mut out, scene, &pic, palette);
    draw_frames(&mut out, scene, &pic, palette, cx);
    let across = scene.arrows.first().is_some_and(|arrow| arrow.across);
    if across {
        draw_arrows(&mut out, scene, &pic, palette);
    }
    draw_cards(&mut out, scene, &pic, palette, cx);
    if !across {
        draw_arrows(&mut out, scene, &pic, palette);
    }
    out.push_str("</svg>");
    out
}

/// Where the picture looks: the camera, on a picture `size` big.
struct Picture {
    camera: Camera,
    size: (f32, f32),
}

impl Picture {
    /// Point `(x, y)` of the drawing, on the picture.
    fn at(&self, (x, y): (f32, f32)) -> (f32, f32) {
        (
            (x - self.camera.x) * self.camera.zoom + self.size.0 / 2.,
            (y - self.camera.y) * self.camera.zoom + self.size.1 / 2.,
        )
    }

    /// `rect` of the drawing on the picture, as x, y, width and height.
    fn rect_of(&self, rect: Rect) -> (f32, f32, f32, f32) {
        let (x, y) = self.at((rect.x, rect.y));
        (
            x,
            y,
            rect.width * self.camera.zoom,
            rect.height * self.camera.zoom,
        )
    }
}

/// The regions: domains, Site's boxes holding others.
fn draw_regions(out: &mut String, scene: &Scene, pic: &Picture, palette: &Palette) {
    let camera = pic.camera;
    for region in &scene.regions {
        let (x, y, w, h) = pic.rect_of(region.rect);
        let quiet = !matches!(region.kind, RegionKind::Domain(_));
        let colour = match region.kind {
            RegionKind::Domain(ix) if !region.draft => domain::colour(ix, palette.dark),
            _ => palette.muted,
        };
        let fill = blend(
            palette.background,
            colour,
            if quiet || region.draft { 0.04 } else { 0.1 },
        );
        let stroke = if region.proposed {
            palette.agent
        } else {
            colour
        };
        let dash = if region.draft || region.kind == RegionKind::Unsorted {
            r#" stroke-dasharray="8 6""#
        } else {
            ""
        };
        let _ = write!(
            out,
            r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="14" fill="{}" stroke="{}" stroke-opacity="{}" stroke-width="{}"{dash}/>"#,
            hex(fill),
            hex(stroke),
            if region.lit { 1. } else { 0.6 },
            if region.lit { 3. } else { 1.5 },
        );
        let name_colour = if quiet || region.draft {
            palette.foreground
        } else {
            blend(colour, palette.foreground, 0.25)
        };
        // An open region's words stay in its header, above what's inside.
        let bottom = if region.open > 0. {
            y + (region.header * camera.zoom).min(h) - 4.
        } else {
            y + h - 8.
        };
        // Its name above it, as long as it is; inside, how much it holds.
        let name = name_label(&region.name, quiet, name_room(w, camera.zoom));
        for (n, line) in name.lines.iter().enumerate() {
            let from_bottom = (name.lines.len() - 1 - n) as f32;
            text(
                out,
                x + 4.,
                y - 8. - from_bottom * name.size * 1.25,
                name.size,
                name_colour,
                true,
                line,
            );
        }
        let mut line_y = y + 4.;
        for line in wrap(&region.note, w - 28., 10.5) {
            line_y += 18.;
            if line_y > bottom + 4. {
                break;
            }
            text(out, x + 14., line_y, 10.5, palette.muted, false, &line);
        }
        let about = if region.draft {
            &region.why
        } else {
            &region.purpose
        };
        line_y += 8.;
        for line in wrap(about, w - 28., 12.5) {
            line_y += 17.;
            if line_y > bottom {
                break;
            }
            text(out, x + 14., line_y, 12.5, palette.foreground, false, &line);
        }
        if !region.offers.is_empty() {
            line_y += 6.;
            for line in wrap(&region.offers.join("  ·  "), w - 28., 10.5) {
                line_y += 15.;
                if line_y > bottom {
                    break;
                }
                text(out, x + 14., line_y, 10.5, palette.muted, false, &line);
            }
        }
    }
}

/// The frames: a Plan's types and files.
fn draw_frames(out: &mut String, scene: &Scene, pic: &Picture, palette: &Palette, cx: &App) {
    let camera = pic.camera;
    for frame in &scene.frames {
        let (x, y, w, h) = pic.rect_of(frame.rect);
        let accent = match frame.tint {
            Some(tint) => domain::colour(tint, palette.dark),
            None => look_colour(frame.look, cx),
        };
        let _ = write!(
            out,
            r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="8" fill="{}" fill-opacity="0.035" stroke="{}" stroke-opacity="0.3"/>"#,
            hex(accent),
            hex(accent),
        );
        let bar = frame.header * camera.zoom;
        let size = if frame.look.fixed_text() {
            (bar * 0.55).min(12.)
        } else {
            11. * camera.zoom
        };
        if frame.look.fixed_text() && bar < 13. {
            continue;
        }
        let baseline = y + bar / 2. + size * 0.35;
        text(
            out,
            x + 10.,
            baseline,
            size,
            palette.foreground,
            true,
            &frame.title,
        );
        let note_x = x + 10. + frame.title.chars().count() as f32 * size * 0.6 + 10.;
        if note_x < x + w - 40. {
            text(
                out,
                note_x,
                baseline,
                (bar * 0.48).min(10.5),
                palette.muted,
                false,
                &frame.note,
            );
        }
    }
}

/// The cards: the boxes that hold nothing drawn.
fn draw_cards(out: &mut String, scene: &Scene, pic: &Picture, palette: &Palette, cx: &App) {
    let camera = pic.camera;
    for card in &scene.cards {
        let (x, y, w, h) = pic.rect_of(card.rect);
        let accent = match card.tint {
            Some(tint) => domain::colour(tint, palette.dark),
            None => look_colour(card.look, cx),
        };
        if card.look.fixed_text() {
            let fill = blend(
                palette.card,
                accent,
                if card.look == Look::Unread {
                    0.04
                } else {
                    0.12
                },
            );
            let _ = write!(
                out,
                r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{}" fill="{}" stroke="{}" stroke-opacity="0.45"/>"#,
                (h / 6.).clamp(2., 7.),
                hex(fill),
                hex(accent),
            );
            if w >= 46. && h >= 16. {
                // As the canvas draws it: the title on two lines at most,
                // what it's for in the lines left, then how much.
                let lines = ((h - 10.) / 14.).floor().max(1.) as usize;
                let title_lines = if lines >= 3 { 2 } else { 1 };
                let detail_lines = lines.saturating_sub(title_lines + 1);
                let mut line_y = y + 4.;
                for line in clamp_lines(&card.title, w - 16., 12., title_lines) {
                    line_y += 14.;
                    text(out, x + 8., line_y, 12., palette.foreground, true, &line);
                }
                for line in clamp_lines(&card.detail, w - 16., 10.5, detail_lines) {
                    line_y += 13.;
                    text(out, x + 8., line_y, 10.5, palette.muted, false, &line);
                }
                if lines >= 2 {
                    text(
                        out,
                        x + 8.,
                        line_y + 14.,
                        10.,
                        palette.muted,
                        false,
                        &clip(&card.note, w - 16., 10.),
                    );
                }
            }
        } else {
            let header = HEADER_HEIGHT * camera.zoom;
            let body = match card.tint {
                Some(tint) => blend(palette.card, domain::colour(tint, palette.dark), 0.14),
                None => palette.card,
            };
            let _ = write!(
                out,
                r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="{r}" fill="{}" stroke="{}" stroke-opacity="0.3"/><rect x="{x}" y="{y}" width="{w}" height="{header}" rx="{r}" fill="{}"/>"#,
                hex(body),
                hex(palette.foreground),
                hex(accent),
                r = 7. * camera.zoom,
            );
            let size = 11.5 * camera.zoom;
            text(
                out,
                x + 8. * camera.zoom,
                y + header * 0.68,
                size,
                gpui_kit::white(),
                true,
                &clip(&card.title, w - 16. * camera.zoom, size),
            );
            let size = 10.5 * camera.zoom;
            let mut line_y = y + header + 16. * camera.zoom;
            for line in [&card.detail, &card.note] {
                if line.is_empty() {
                    continue;
                }
                text(
                    out,
                    x + 10. * camera.zoom,
                    line_y,
                    size,
                    palette.muted,
                    false,
                    &clip(line, w - 20. * camera.zoom, size),
                );
                line_y += ROW_HEIGHT * camera.zoom;
            }
        }
    }
}
