//! Export Image: the level showing as a picture, to copy or save. Domain
//! and Plan are drawn whole, Site as it is in view. It's drawn from the
//! same scene the canvas draws, as SVG, and turned into a PNG (or saved as
//! the SVG, for a name ending in `.svg`).

use std::fmt::Write as _;
use std::time::Instant;

use anyhow::{Context as _, Result};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

use super::super::domains::level::RegionKind;
use super::super::layout::{HEADER_HEIGHT, Rect};
use super::draw::{blend, bow_point, look_colour, theme};
use super::scene::{Look, Scene};
use super::zoom::Camera;
use super::{Level, MapView, domain};

/// The longest side of an exported picture, before it's doubled for
/// sharpness.
const MAX_SIDE: f32 = 3200.;
const MARGIN: f32 = 40.;
const FONT: &str = "-apple-system, 'Helvetica Neue', Helvetica, Arial, sans-serif";

/// The theme's colours, as the picture uses them.
struct Palette {
    background: Hsla,
    foreground: Hsla,
    muted: Hsla,
    card: Hsla,
    agent: Hsla,
    dark: bool,
}

impl MapView {
    /// Copy Image: the level showing, as a PNG on the clipboard.
    pub(super) fn copy_image(&mut self, cx: &mut Context<Self>) {
        let svg = self.export_svg(cx);
        match png(&svg) {
            Ok(bytes) => {
                cx.write_to_clipboard(ClipboardItem::new_image(&Image::from_bytes(
                    ImageFormat::Png,
                    bytes,
                )));
                self.say(&format!("Copied an image of {}.", self.level_title()), cx);
            }
            Err(error) => self.say(&format!("Couldn't make the image: {error:#}"), cx),
        }
    }

    /// Save Image…: the level showing, as a PNG (or an SVG) where the user
    /// says.
    pub(super) fn save_image(&mut self, cx: &mut Context<Self>) {
        let svg = self.export_svg(cx);
        let project = self
            .root
            .file_name()
            .map_or("Project".into(), |name| name.to_string_lossy().into_owned());
        let name = format!("{project} {}.png", self.level.name());
        let path = cx.prompt_for_new_path(&self.root, Some(&name));
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            let written = if path.extension().is_some_and(|ext| ext == "svg") {
                std::fs::write(&path, svg).context("writing the image")
            } else {
                png(&svg)
                    .and_then(|bytes| std::fs::write(&path, bytes).context("writing the image"))
            };
            this.update(cx, |this, cx| match written {
                Ok(()) => this.say(&format!("Saved {}.", path.display()), cx),
                Err(error) => this.say(&format!("Couldn't save the image: {error:#}"), cx),
            })
            .ok();
        })
        .detach();
    }

    fn level_title(&self) -> &'static str {
        match self.level {
            Level::Plan(_) => "the Plan",
            Level::Site => "Site",
            Level::Domain => "the domains",
        }
    }

    /// The level showing as SVG.
    pub fn export_svg(&self, cx: &App) -> String {
        let theme = cx.theme();
        let palette = Palette {
            background: theme::canvas(cx),
            foreground: theme.foreground,
            muted: theme.muted_foreground,
            card: theme.background,
            agent: jig_commands::surface::agent_accent(cx),
            dark: theme.mode.is_dark(),
        };
        let (camera, size) = match self.level {
            Level::Site => (self.camera(), self.canvas_size()),
            _ => {
                let bounds = self.content_rect().unwrap_or(Rect {
                    x: 0.,
                    y: 0.,
                    width: 400.,
                    height: 300.,
                });
                let zoom = (MAX_SIDE / bounds.width.max(bounds.height).max(1.)).min(1.2);
                let size = (
                    bounds.width * zoom + 2. * MARGIN,
                    bounds.height * zoom + 2. * MARGIN,
                );
                let (x, y) = bounds.centre();
                (Camera { x, y, zoom }, size)
            }
        };
        let mut scene = self.scene(camera, Instant::now());
        if self.level == Level::Domain {
            // The domains as an overview: their words, not their files.
            scene.cards.clear();
            for region in &mut scene.regions {
                region.open = 0.;
            }
        }
        draw_svg(&scene, camera, size, &palette, cx)
    }
}

/// The SVG of `scene` seen by `camera` on a picture `size` big.
fn draw_svg(
    scene: &Scene,
    camera: Camera,
    size: (f32, f32),
    palette: &Palette,
    cx: &App,
) -> String {
    let at = |(x, y): (f32, f32)| {
        (
            (x - camera.x) * camera.zoom + size.0 / 2.,
            (y - camera.y) * camera.zoom + size.1 / 2.,
        )
    };
    let rect_of = |rect: Rect| {
        let (x, y) = at((rect.x, rect.y));
        (x, y, rect.width * camera.zoom, rect.height * camera.zoom)
    };
    let mut out = String::new();
    let _ = write!(
        out,
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{w}" height="{h}" viewBox="0 0 {w} {h}" font-family="{FONT}"><rect width="{w}" height="{h}" fill="{bg}"/>"#,
        w = size.0.ceil(),
        h = size.1.ceil(),
        bg = hex(palette.background),
    );
    for region in &scene.regions {
        let (x, y, w, h) = rect_of(region.rect);
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
            r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" rx="14" fill="{}" stroke="{}" stroke-opacity="0.6" stroke-width="1.5"{dash}/>"#,
            hex(fill),
            hex(stroke),
        );
        let name_colour = if quiet || region.draft {
            palette.foreground
        } else {
            blend(colour, palette.foreground, 0.25)
        };
        let mut line_y = y + 30.;
        let size = if quiet { 13. } else { 16. };
        text(
            &mut out,
            x + 14.,
            line_y,
            size,
            name_colour,
            true,
            &region.name,
        );
        let note_x = x + 14. + region.name.chars().count() as f32 * size * 0.58 + 10.;
        text(
            &mut out,
            note_x,
            line_y,
            10.5,
            palette.muted,
            false,
            &region.note,
        );
        let about = if region.draft {
            &region.why
        } else {
            &region.purpose
        };
        line_y += 8.;
        for line in wrap(about, w - 28., 12.5) {
            line_y += 17.;
            if line_y > y + h - 8. {
                break;
            }
            text(
                &mut out,
                x + 14.,
                line_y,
                12.5,
                palette.foreground,
                false,
                &line,
            );
        }
        if !region.offers.is_empty() {
            line_y += 6.;
            for line in wrap(&region.offers.join("  ·  "), w - 28., 10.5) {
                line_y += 15.;
                if line_y > y + h - 8. {
                    break;
                }
                text(&mut out, x + 14., line_y, 10.5, palette.muted, false, &line);
            }
        }
    }
    for frame in &scene.frames {
        let (x, y, w, h) = rect_of(frame.rect);
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
        let size = if frame.look.fixed_text() {
            12.
        } else {
            11. * camera.zoom
        };
        text(
            &mut out,
            x + 10.,
            y + 6. + size,
            size,
            palette.foreground,
            true,
            &frame.title,
        );
    }
    let paint_arrows = |out: &mut String| {
        for arrow in &scene.arrows {
            let (from, to) = (at(arrow.from), at(arrow.to));
            let colour = palette.foreground;
            let width = match &arrow.calls {
                Some(calls) => (1. + (calls.count as f32).sqrt() * 0.9).clamp(1.2, 7.),
                None if !arrow.labels.is_empty() => 2.,
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
                let lines: Vec<String> = arrow
                    .labels
                    .iter()
                    .map(|(forward, label)| {
                        let (dx, dy) = if *forward {
                            (to.0 - from.0, to.1 - from.1)
                        } else {
                            (from.0 - to.0, from.1 - to.1)
                        };
                        let glyph = if dx.abs() >= dy.abs() {
                            if dx >= 0. { "→" } else { "←" }
                        } else if dy >= 0. {
                            "↓"
                        } else {
                            "↑"
                        };
                        format!("{glyph} {label}")
                    })
                    .collect();
                let width = lines
                    .iter()
                    .map(|line| line.chars().count() as f32 * 10.5 * 0.55)
                    .fold(0., f32::max)
                    + 12.;
                let height = lines.len() as f32 * 14. + 6.;
                let (x, y) = (middle.0 - width / 2., middle.1 - 10.);
                let _ = write!(
                    out,
                    r#"<rect x="{x}" y="{y}" width="{width}" height="{height}" rx="6" fill="{}" stroke="{}" stroke-opacity="0.18"/>"#,
                    hex(palette.background),
                    hex(palette.foreground),
                );
                for (n, line) in lines.iter().enumerate() {
                    text(
                        out,
                        x + 6.,
                        y + 14. + n as f32 * 14.,
                        10.5,
                        palette.foreground,
                        false,
                        line,
                    );
                }
            }
        }
    };
    let across = scene.arrows.first().is_some_and(|arrow| arrow.across);
    if across {
        paint_arrows(&mut out);
    }
    for card in &scene.cards {
        let (x, y, w, h) = rect_of(card.rect);
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
                text(
                    &mut out,
                    x + 8.,
                    y + 15.,
                    12.,
                    palette.foreground,
                    true,
                    &clip(&card.title, w - 16., 12.),
                );
            }
            if h >= 38. && !card.detail.is_empty() {
                text(
                    &mut out,
                    x + 8.,
                    y + 30.,
                    10.5,
                    palette.muted,
                    false,
                    &clip(&card.detail, w - 16., 10.5),
                );
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
                &mut out,
                x + 8. * camera.zoom,
                y + header * 0.68,
                size,
                gpui_kit::white(),
                true,
                &clip(&card.title, w - 16. * camera.zoom, size),
            );
            if !card.detail.is_empty() {
                let size = 10.5 * camera.zoom;
                text(
                    &mut out,
                    x + 10. * camera.zoom,
                    y + header + 16. * camera.zoom,
                    size,
                    palette.muted,
                    false,
                    &clip(&card.detail, w - 20. * camera.zoom, size),
                );
            }
        }
    }
    if !across {
        paint_arrows(&mut out);
    }
    out.push_str("</svg>");
    out
}

/// The SVG made a PNG, twice its size for sharpness.
fn png(svg: &str) -> Result<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let mut options = usvg::Options::default();
    options.fontdb_mut().load_system_fonts();
    let tree = usvg::Tree::from_str(svg, &options).context("drawing the image")?;
    let scale = 2.;
    let size = tree
        .size()
        .to_int_size()
        .scale_by(scale)
        .context("sizing the image")?;
    let mut pixmap =
        tiny_skia::Pixmap::new(size.width(), size.height()).context("the image is too big")?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().context("encoding the image")
}

fn bow(from: (f32, f32), to: (f32, f32)) -> (f32, f32) {
    let middle = bow_point(
        point(px(from.0), px(from.1)),
        point(px(to.0), px(to.1)),
        0.5,
    );
    (f32::from(middle.x), f32::from(middle.y))
}

fn head(out: &mut String, middle: (f32, f32), to: (f32, f32), width: f32, colour: Hsla) {
    let (dx, dy) = (to.0 - middle.0, to.1 - middle.1);
    let length = dx.hypot(dy).max(0.001);
    let (ux, uy) = (dx / length, dy / length);
    let size = 5. + width * 1.5;
    let back = (to.0 - ux * size, to.1 - uy * size);
    let side = (-uy * size * 0.55, ux * size * 0.55);
    let _ = write!(
        out,
        r#"<path d="M{} {} L{} {} L{} {} Z" fill="{}" fill-opacity="0.5"/>"#,
        to.0,
        to.1,
        back.0 + side.0,
        back.1 + side.1,
        back.0 - side.0,
        back.1 - side.1,
        hex(colour),
    );
}

fn text(out: &mut String, x: f32, y: f32, size: f32, colour: Hsla, bold: bool, words: &str) {
    if words.is_empty() {
        return;
    }
    let _ = write!(
        out,
        r#"<text x="{x}" y="{y}" font-size="{size}" fill="{}"{}>{}</text>"#,
        hex(colour),
        if bold { r#" font-weight="600""# } else { "" },
        escape(words),
    );
}

/// `text` in lines that fit `width` at `size`, roughly.
fn wrap(text: &str, width: f32, size: f32) -> Vec<String> {
    let per_line = ((width / (size * 0.52)) as usize).max(8);
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > per_line {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// `text` cut to fit `width` at `size`, roughly.
fn clip(text: &str, width: f32, size: f32) -> String {
    let fits = ((width / (size * 0.55)) as usize).max(1);
    if text.chars().count() <= fits {
        text.to_string()
    } else {
        let cut: String = text.chars().take(fits.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn hex(colour: Hsla) -> String {
    let rgba = Rgba::from(colour);
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(rgba.r),
        byte(rgba.g),
        byte(rgba.b)
    )
}

#[cfg(test)]
mod tests {
    use super::{clip, escape, png, wrap};

    #[test]
    fn long_words_wrap_and_markup_is_escaped() {
        assert_eq!(
            wrap("Book a class and pay for it online", 120., 12.5),
            ["Book a class and", "pay for it online"]
        );
        assert_eq!(escape("<a & b>"), "&lt;a &amp; b&gt;");
        assert_eq!(clip("abcdefghijklmnop", 40., 10.), "abcdef…");
    }

    #[test]
    fn an_svg_becomes_a_png() {
        let bytes = png(r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="#336699"/></svg>"##).unwrap();
        assert_eq!(&bytes[1..4], b"PNG");
    }
}
