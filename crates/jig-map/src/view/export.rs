//! Export Image: the level showing as a picture, to copy or save. Domain
//! and Plan are drawn whole, Site as it is in view. It's drawn from the
//! same scene the canvas draws, as SVG, and turned into a PNG (or saved as
//! the SVG, for a name ending in `.svg`). The agent's drawings come too,
//! with a list of them.

use std::fmt::Write as _;
use std::time::Instant;

use anyhow::{Context as _, Result};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

use super::super::domains::level::RegionKind;
use super::super::layout::{HEADER_HEIGHT, ROW_HEIGHT, Rect};
use super::domain_draw::{glyph, name_label, name_room};
use super::draw::{
    blend, bow_point, flow_label_size, label_spot, leader, look_colour, obstacles, theme,
};
use super::lens_draw::Picture;
use super::scene::{Look, Scene};
use super::sketch_draw::{BoxState, SketchPicture, note_colours, short_place};
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
                let bounds = Rect::around(self.content_rect().into_iter().chain(
                    // Room for the sketches' bars above them.
                    self.sketches_bounds().map(|r| Rect {
                        y: r.y - 60.,
                        height: r.height + 60.,
                        ..r
                    }),
                ))
                .unwrap_or(Rect {
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
        // A picture to share: every flow in words.
        for arrow in &mut scene.arrows {
            arrow.faint = false;
        }
        if self.level == Level::Domain {
            // The domains as an overview: their words, not their files.
            scene.cards.clear();
            for region in &mut scene.regions {
                region.open = 0.;
            }
        }
        let svg = draw_svg(&scene, camera, size, &palette, cx);
        let svg = self.with_sketches(svg, camera, size, &palette);
        self.with_lens(svg, camera, size, &palette)
    }

    /// `svg` with the user's sketches over it.
    fn with_sketches(
        &self,
        svg: String,
        camera: Camera,
        size: (f32, f32),
        palette: &Palette,
    ) -> String {
        let screen = |rect: Rect| Rect {
            x: (rect.x - camera.x) * camera.zoom + size.0 / 2.,
            y: (rect.y - camera.y) * camera.zoom + size.1 / 2.,
            width: rect.width * camera.zoom,
            height: rect.height * camera.zoom,
        };
        let picture = self.sketch_picture(&screen);
        let marks = self.sketch_marks();
        if picture.is_empty() && marks.is_empty() {
            return svg;
        }
        let mut out = svg.trim_end_matches("</svg>").to_string();
        draw_sketches(&mut out, &picture, palette);
        // The sketches of other levels, marked on the boxes carrying them.
        let ink = hex(palette.foreground);
        for (landing, names) in marks {
            let rect = screen(landing.rect);
            let titles: Vec<String> = names
                .iter()
                .filter_map(|name| self.sketch(name).map(|s| s.title.clone()))
                .collect();
            let label = match titles.as_slice() {
                [one] => format!("✎ {one}"),
                many => format!("✎ {} sketches", many.len()),
            };
            let width = label.chars().count() as f32 * 6.6 + 16.;
            let (x, y) = (rect.right() - 8., rect.y - 10.);
            let _ = write!(
                out,
                r#"<rect x="{x}" y="{y}" width="{width}" height="20" rx="10" fill="{}" stroke="{ink}" stroke-opacity="0.8" stroke-dasharray="4 3"/>"#,
                hex(palette.card)
            );
            text(
                &mut out,
                x + 8.,
                y + 14.,
                11.,
                palette.foreground,
                false,
                &label,
            );
        }
        out.push_str("</svg>");
        out
    }

    /// `svg` with the agent's drawings over it, and a list of them.
    fn with_lens(
        &self,
        svg: String,
        camera: Camera,
        size: (f32, f32),
        palette: &Palette,
    ) -> String {
        if self.drawings_showing().is_empty() {
            return svg;
        }
        let screen = |rect: Rect| Rect {
            x: (rect.x - camera.x) * camera.zoom + size.0 / 2.,
            y: (rect.y - camera.y) * camera.zoom + size.1 / 2.,
            width: rect.width * camera.zoom,
            height: rect.height * camera.zoom,
        };
        let picture = self.lens_picture(&screen);
        let list = self.lens_list();
        let band = 24. + list.len() as f32 * 17.;
        // The list goes in a band below the picture, covering nothing.
        let (w, h) = (size.0.ceil(), size.1.ceil());
        let taller = h + band;
        let mut out = svg.trim_end_matches("</svg>").replacen(
            &format!(r#"height="{h}" viewBox="0 0 {w} {h}""#),
            &format!(r#"height="{taller}" viewBox="0 0 {w} {taller}""#),
            1,
        );
        let _ = write!(
            out,
            r#"<rect x="0" y="{h}" width="{w}" height="{band}" fill="{}"/>"#,
            hex(palette.card)
        );
        draw_lens(&mut out, &picture, size, palette);
        for (n, (line, heading)) in list.iter().enumerate() {
            let colour = if *heading {
                palette.agent
            } else {
                palette.foreground
            };
            text(
                &mut out,
                16.,
                h + 22. + n as f32 * 17.,
                11.5,
                colour,
                *heading,
                &clip(line, w - 32., 11.5),
            );
        }
        out.push_str("</svg>");
        out
    }

    /// The list of drawings: each, with its steps.
    fn lens_list(&self) -> Vec<(String, bool)> {
        let mut lines: Vec<(String, bool)> = Vec::new();
        for (_, drawing) in self.drawings_showing() {
            lines.push((drawing.summary(), true));
            if drawing.kind.stepped() {
                for (ix, step) in drawing.steps.iter().enumerate() {
                    let text = format!(
                        "{}. {} — {}",
                        ix + 1,
                        step.label,
                        self.short_name(&step.place)
                    );
                    lines.push((text, false));
                }
            }
        }
        lines
    }
}

/// The agent's drawings, as the canvas draws them, in SVG.
fn draw_lens(out: &mut String, picture: &Picture, size: (f32, f32), palette: &Palette) {
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
            let (x, y, arrow) = super::lens_draw::edge_spot(r, size);
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
        let lines: Vec<String> = super::lens_draw::capped(&marked.words)
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

/// The user's sketches, as the canvas draws them, in SVG: neutral ink,
/// dashed while they're ideas, solid once real.
fn draw_sketches(out: &mut String, picture: &SketchPicture, palette: &Palette) {
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

fn palette_green(palette: &Palette) -> Hsla {
    if palette.dark {
        hsla(140. / 360., 0.5, 0.6, 1.)
    } else {
        hsla(140. / 360., 0.6, 0.32, 1.)
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
                &mut out,
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
            text(&mut out, x + 14., line_y, 10.5, palette.muted, false, &line);
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
                if line_y > bottom {
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
            &mut out,
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
                &mut out,
                note_x,
                baseline,
                (bar * 0.48).min(10.5),
                palette.muted,
                false,
                &frame.note,
            );
        }
    }
    let mut avoid = obstacles(scene, |rect| {
        let (x, y, width, height) = rect_of(rect);
        Rect {
            x,
            y,
            width,
            height,
        }
    });
    let size_of_picture = size;
    let mut paint_arrows = |out: &mut String| {
        for arrow in &scene.arrows {
            let (from, to) = (at(arrow.from), at(arrow.to));
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
                // As the canvas draws it: the title on two lines at most,
                // what it's for in the lines left, then how much.
                let lines = ((h - 10.) / 14.).floor().max(1.) as usize;
                let title_lines = if lines >= 3 { 2 } else { 1 };
                let detail_lines = lines.saturating_sub(title_lines + 1);
                let mut line_y = y + 4.;
                for line in clamp_lines(&card.title, w - 16., 12., title_lines) {
                    line_y += 14.;
                    text(
                        &mut out,
                        x + 8.,
                        line_y,
                        12.,
                        palette.foreground,
                        true,
                        &line,
                    );
                }
                for line in clamp_lines(&card.detail, w - 16., 10.5, detail_lines) {
                    line_y += 13.;
                    text(&mut out, x + 8., line_y, 10.5, palette.muted, false, &line);
                }
                if lines >= 2 {
                    text(
                        &mut out,
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
                &mut out,
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
                    &mut out,
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

/// `text` wrapped to fit `width` at `size`, at most `most` lines, the
/// last cut short if there's more.
fn clamp_lines(text: &str, width: f32, size: f32, most: usize) -> Vec<String> {
    let mut lines = wrap(text, width, size);
    if lines.len() > most {
        lines.truncate(most);
        if let Some(last) = lines.last_mut() {
            *last = clip(&format!("{last} …"), width, size);
        }
    }
    lines
        .into_iter()
        .map(|line| clip(&line, width, size))
        .collect()
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

#[cfg(any(test, feature = "testing"))]
impl MapView {
    /// For `map_pictures_live`: the level showing at several zooms around where the
    /// camera is (fitted whole, as it is, and closer), as PNGs.
    pub fn pictures(&mut self, cx: &App) -> Vec<(String, Vec<u8>)> {
        self.transition = None;
        let size = self.canvas_size();
        let here = self.camera();
        let mut cameras = vec![("here".to_string(), here)];
        if let Some(whole) = self.content_rect() {
            let screen = Rect {
                x: MARGIN,
                y: MARGIN,
                width: size.0 - 2. * MARGIN,
                height: size.1 - 2. * MARGIN,
            };
            cameras.insert(0, ("whole".into(), Camera::showing(whole, screen, size)));
        }
        // The way from the whole to here, in even steps of zoom.
        if let Some((_, whole)) = cameras.first().cloned() {
            let steps = 7;
            for step in 1..steps {
                let t = step as f32 / steps as f32;
                let zoom = whole.zoom * (here.zoom / whole.zoom).powf(t);
                // Zooming about where it lands, as a pinch there would.
                cameras.push((format!("step{step}"), Camera { zoom, ..here }));
            }
        }
        let mut out = Vec::new();
        for (name, camera) in cameras {
            self.set_camera(camera);
            let svg = self.export_svg(cx);
            if let Ok(bytes) = png(&svg) {
                out.push((name, bytes));
            }
        }
        self.set_camera(here);
        out
    }

    /// For `map_pictures_live`: the canvas as it is now, as a PNG.
    pub fn picture_now(&mut self, cx: &App) -> Option<Vec<u8>> {
        self.transition = None;
        let palette = Palette {
            background: theme::canvas(cx),
            foreground: cx.theme().foreground,
            muted: cx.theme().muted_foreground,
            card: cx.theme().background,
            agent: jig_commands::surface::agent_accent(cx),
            dark: cx.theme().mode.is_dark(),
        };
        let camera = self.camera();
        let scene = self.scene(camera, Instant::now());
        let svg = draw_svg(&scene, camera, self.canvas_size(), &palette, cx);
        let svg = self.with_sketches(svg, camera, self.canvas_size(), &palette);
        png(&self.with_lens(svg, camera, self.canvas_size(), &palette)).ok()
    }
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
