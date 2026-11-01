//! Sizing Site's boxes by what they hold and placing the regions.

use super::*;

/// Boxes of these sizes in rows, as near `aspect` as they pack: where
/// each goes, and how wide and tall they come to.
pub(super) fn shelves(sizes: &[(f32, f32)], gap: f32, aspect: f32) -> (Vec<(f32, f32)>, f32, f32) {
    let area: f32 = sizes.iter().map(|(w, h)| (w + gap) * (h + gap)).sum();
    let widest = sizes.iter().map(|(w, _)| *w).fold(0., f32::max);
    let row = (area * aspect).sqrt().max(widest);
    let mut spots = Vec::with_capacity(sizes.len());
    let (mut x, mut y, mut tallest, mut width) = (0., 0., 0f32, 0f32);
    for &(w, h) in sizes {
        if x > 0. && x + w > row {
            x = 0.;
            y += tallest + gap;
            tallest = 0.;
        }
        spots.push((x, y));
        x += w + gap;
        width = width.max(x - gap);
        tallest = tallest.max(h);
    }
    (spots, width, y + tallest)
}

/// A box wide enough for its title, at the size Site draws titles.
pub(super) fn fitted(title: &str, at_least: f32) -> f32 {
    (title.chars().count() as f32 * 7.4 + 26.).clamp(at_least, FILE_MAX_WIDTH)
}

impl Site {
    /// Each box's count of what it holds, its detail line and what's in
    /// it by name.
    pub(super) fn count(&mut self, ix: usize) -> (usize, usize) {
        let children = self.nodes[ix].children.clone();
        let node_kind = self.nodes[ix].kind;
        let (files, functions) = match node_kind {
            Kind::Type => (0, self.nodes[ix].functions),
            Kind::File { .. } => {
                for &child in &children {
                    self.count(child);
                }
                (1, self.nodes[ix].functions)
            }
            _ => {
                let (mut files, mut functions) = (0, 0);
                for &child in &children {
                    let (f, n) = self.count(child);
                    files += f;
                    functions += n;
                }
                self.nodes[ix].functions = functions;
                (files, functions)
            }
        };
        self.nodes[ix].detail = match node_kind {
            Kind::Type => plural(functions, "function"),
            Kind::File { read: false } => "not read".into(),
            Kind::File { .. } => plural(functions, "function"),
            _ if functions > 0 => format!(
                "{} · {}",
                plural(files, "file"),
                plural(functions, "function")
            ),
            _ => plural(files, "file"),
        };
        let names: Vec<String> = children
            .iter()
            .map(|&c| self.nodes[c].title.clone())
            .collect();
        self.nodes[ix].contents = summary(&names);
        (files, functions)
    }

    /// Node `ix`'s size, from what's in it, its children placed relative
    /// to it.
    pub(super) fn size(&mut self, ix: usize) {
        let children = self.nodes[ix].children.clone();
        for &child in &children {
            self.size(child);
        }
        let kind = self.nodes[ix].kind;
        let (width, height) = match kind {
            Kind::Root => (0., 0.),
            Kind::Type => (fitted(&self.nodes[ix].title, TYPE_WIDTH), TYPE_HEIGHT),
            Kind::File { .. } if children.is_empty() => {
                (fitted(&self.nodes[ix].title, FILE_WIDTH), FILE_HEIGHT)
            }
            _ => {
                let region = matches!(kind, Kind::Region(_));
                let pad = if region { REGION_PAD } else { PAD };
                let sizes: Vec<(f32, f32)> = children
                    .iter()
                    .map(|&c| (self.nodes[c].rect.width, self.nodes[c].rect.height))
                    .collect();
                let (spots, inner_width, inner_height) = shelves(&sizes, GAP, ASPECT);
                let minimum = if region {
                    REGION_MIN_WIDTH
                } else {
                    fitted(&self.nodes[ix].title, FILE_WIDTH) + 2. * pad
                };
                let width = (inner_width + 2. * pad).max(minimum);
                let header = if region {
                    REGION_HEADER.max(width * 0.11)
                } else {
                    TITLE.max(width * 0.045)
                };
                self.nodes[ix].header = header;
                for (&child, (x, y)) in children.iter().zip(spots) {
                    self.nodes[child].rect.x = pad + x;
                    self.nodes[child].rect.y = header + y;
                }
                let height = header + inner_height + pad;
                let height = if region {
                    height.max(width / ASPECT * 0.55)
                } else {
                    height
                };
                (width, height)
            }
        };
        self.nodes[ix].rect.width = width;
        self.nodes[ix].rect.height = height;
    }

    /// The domains in rows, as near `ASPECT` as they pack, Shared and
    /// Unsorted apart below them; the project's box round them all.
    pub(super) fn place_regions(&mut self) {
        let regions: Vec<usize> = self.regions().collect();
        let (domains, apart): (Vec<usize>, Vec<usize>) = regions
            .into_iter()
            .partition(|&ix| matches!(self.nodes[ix].kind, Kind::Region(Home::Domain(_))));
        let sizes: Vec<(f32, f32)> = domains
            .iter()
            .map(|&ix| (self.nodes[ix].rect.width, self.nodes[ix].rect.height))
            .collect();
        let (spots, _, height) = shelves(&sizes, REGION_GAP, REGIONS_ASPECT);
        for (&ix, (x, y)) in domains.iter().zip(spots) {
            self.nodes[ix].rect.x = x;
            self.nodes[ix].rect.y = y;
        }
        let below = if domains.is_empty() {
            0.
        } else {
            height + APART
        };
        let mut x = 0.;
        for ix in apart {
            self.nodes[ix].rect.x = x;
            self.nodes[ix].rect.y = below;
            x += self.nodes[ix].rect.width + REGION_GAP;
        }
        let bounds = Rect::around(self.regions().map(|ix| self.nodes[ix].rect)).unwrap_or_default();
        self.nodes[0].rect = bounds;
        // Children are placed relative to the project's corner.
        for ix in self.regions().collect::<Vec<_>>() {
            self.nodes[ix].rect.x -= bounds.x;
            self.nodes[ix].rect.y -= bounds.y;
        }
        self.nodes[0].rect.x = 0.;
        self.nodes[0].rect.y = 0.;
        let _ = bounds;
    }

    /// Node `ix` moved by `(x, y)` from where its parent put it, and
    /// what's in it along with it.
    pub(super) fn position(&mut self, ix: usize, x: f32, y: f32) {
        if ix != 0 {
            self.nodes[ix].rect.x += x;
            self.nodes[ix].rect.y += y;
        }
        let (x, y) = (self.nodes[ix].rect.x, self.nodes[ix].rect.y);
        for child in self.nodes[ix].children.clone() {
            self.position(child, x, y);
        }
    }

    /// The zoom each box starts opening at: when it's wide enough on
    /// screen, and a typical box inside it would read.
    pub(super) fn open_points(&mut self) {
        for ix in 1..self.nodes.len() {
            let node = &self.nodes[ix];
            if node.children.is_empty() {
                continue;
            }
            let mut widths: Vec<f32> = node
                .children
                .iter()
                .map(|&c| self.nodes[c].rect.width)
                .collect();
            widths.sort_by(f32::total_cmp);
            let typical = widths[widths.len() / 2].max(1.);
            let from = if matches!(node.kind, Kind::Region(_)) {
                REGION_OPEN
            } else {
                OPEN_FROM
            };
            let width = node.rect.width.max(1.);
            self.nodes[ix].open_at = (from / width).max((READ / typical).min(OPEN_BY / width));
        }
    }
}
