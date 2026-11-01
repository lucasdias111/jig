//! Pictures of the Map for the live tests that check its design.

use super::*;

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
