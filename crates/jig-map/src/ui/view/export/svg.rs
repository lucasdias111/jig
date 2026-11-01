//! Writing SVG: arrows, heads, text wrapped and clipped, colours, and the PNG.

use super::*;

/// The SVG made a PNG, twice its size for sharpness.
pub(super) fn png(svg: &str) -> Result<Vec<u8>> {
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

pub(super) fn bow(from: (f32, f32), to: (f32, f32)) -> (f32, f32) {
    let middle = bow_point(
        point(px(from.0), px(from.1)),
        point(px(to.0), px(to.1)),
        0.5,
    );
    (f32::from(middle.x), f32::from(middle.y))
}

pub(super) fn head(out: &mut String, middle: (f32, f32), to: (f32, f32), width: f32, colour: Hsla) {
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

pub(super) fn text(
    out: &mut String,
    x: f32,
    y: f32,
    size: f32,
    colour: Hsla,
    bold: bool,
    words: &str,
) {
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
pub(super) fn wrap(text: &str, width: f32, size: f32) -> Vec<String> {
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
pub(super) fn clamp_lines(text: &str, width: f32, size: f32, most: usize) -> Vec<String> {
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
pub(super) fn clip(text: &str, width: f32, size: f32) -> String {
    let fits = ((width / (size * 0.55)) as usize).max(1);
    if text.chars().count() <= fits {
        text.to_string()
    } else {
        let cut: String = text.chars().take(fits.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

pub(super) fn hex(colour: Hsla) -> String {
    let rgba = Rgba::from(colour);
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    format!(
        "#{:02x}{:02x}{:02x}",
        byte(rgba.r),
        byte(rgba.g),
        byte(rgba.b)
    )
}
