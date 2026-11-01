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

use super::domain_draw::{glyph, name_label, name_room};
use super::draw::{
    blend, bow_point, flow_label_size, label_spot, leader, look_colour, obstacles, theme,
};
use super::lens_draw::Picture;
use super::scene::{Look, Scene};
use super::sketch_draw::{BoxState, SketchPicture, note_colours, short_place};
use super::zoom::Camera;
use super::{Level, MapView, domain};
use crate::domain::domains::level::RegionKind;
use crate::domain::layout::{HEADER_HEIGHT, ROW_HEIGHT, Rect};
use crate::store::persist;
mod lens;
mod level;
mod sketches;
mod svg;
#[cfg(any(test, feature = "testing"))]
mod testing;
use lens::*;
use level::*;
use sketches::*;
use svg::*;

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
                persist::write_image(&path, svg)
            } else {
                png(&svg).and_then(|bytes| persist::write_image(&path, bytes))
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

fn palette_green(palette: &Palette) -> Hsla {
    if palette.dark {
        hsla(140. / 360., 0.5, 0.6, 1.)
    } else {
        hsla(140. / 360., 0.6, 0.32, 1.)
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests;
