//! Site's scene: its boxes as far as they're open, and their arrows.

use super::*;

impl MapView {
    /// Site as `camera` sees it: the regions, those grown big enough open
    /// onto their share of the code, every box inside showing as a frame
    /// with what's inside fading in once that would read; the arrows
    /// between boxes side by side, calls leaving a region as stubs at its
    /// edge, and the domains' flows between regions still closed.
    pub(super) fn site_scene(&self, camera: Camera) -> Scene {
        let mut scene = Scene::default();
        let (Some(site), Some(level)) = (self.site.as_ref(), self.domain_level()) else {
            return scene;
        };
        let (width, height) = self.canvas_size();
        let view = Rect {
            x: camera.x - width / 2. / camera.zoom,
            y: camera.y - height / 2. / camera.zoom,
            width: width / camera.zoom,
            height: height / camera.zoom,
        };
        let zoom = camera.zoom;
        let tints = self.has_tints() && !self.domains.tints_off;
        // How opaque each box is: what's inside a box opening fades in.
        let mut alpha = vec![0f32; site.nodes.len()];
        let mut stack: Vec<(usize, f32)> = Vec::new();
        for region in site.regions() {
            let node = &site.nodes[region];
            let Some(at) = level.regions.iter().position(|r| r.key == node.key) else {
                continue;
            };
            let open = node.opening(zoom);
            scene.regions.push(self.region_box(&level, at, open));
            alpha[region] = 1.;
            if open > 0. && node.rect.overlaps(&view) {
                for &child in node.children.iter().rev() {
                    stack.push((child, open));
                }
            }
        }
        while let Some((ix, shown)) = stack.pop() {
            let node = &site.nodes[ix];
            alpha[ix] = shown;
            if !node.rect.overlaps(&view) {
                continue;
            }
            let open = node.opening(zoom);
            let look = match node.kind {
                Kind::Component => Look::Package,
                Kind::Folder | Kind::Root | Kind::Region(_) => Look::Folder,
                Kind::File { read: true } => Look::File,
                Kind::File { read: false } => Look::Unread,
                Kind::Type => Look::Type,
            };
            let tint = match site.nodes[node.region].kind {
                Kind::Region(Home::Domain(domain)) if tints => Some(domain),
                _ => None,
            };
            let about = node.purpose.clone().unwrap_or_else(|| {
                if matches!(node.kind, Kind::Folder | Kind::Component) {
                    node.contents.clone()
                } else {
                    String::new()
                }
            });
            if open > 0. {
                scene.frames.push(FrameBox {
                    key: node.key.clone(),
                    rect: node.rect,
                    title: node.title.clone(),
                    detail: node.purpose.clone().unwrap_or_default(),
                    note: node.detail.clone(),
                    look,
                    alpha: shown * open,
                    tint,
                    ix,
                    header: node.header,
                });
                for &child in node.children.iter().rev() {
                    stack.push((child, shown * open));
                }
            }
            if open < 1. {
                scene.cards.push(Card {
                    key: node.key.clone(),
                    rect: node.rect,
                    title: node.title.clone(),
                    detail: about,
                    note: node.detail.clone(),
                    badges: Vec::new(),
                    look,
                    alpha: shown * (1. - open),
                    focus: false,
                    hidden: [0; 3],
                    unknown_target: false,
                    checked: false,
                    tint,
                    ix,
                });
            }
        }
        // Inner boxes after outer ones, so they're drawn over them.
        scene.cards.sort_by_key(|card| card.ix);
        scene.frames.sort_by_key(|frame| frame.ix);
        // Arrows join the boxes showing, as a box is counted open half way.
        let shown = site.shown_as(|ix| site.nodes[ix].opening(zoom) >= 0.5);
        let arrows = self.site_arrows(site, &shown);
        // The box selected, as the boxes showing stand for it: its arrows
        // lit, the rest fainter.
        let chosen: Option<usize> = self
            .selected
            .iter()
            .find_map(|key| site.index_of(key.strip_prefix("site:")?))
            .filter(|&ix| !matches!(site.nodes[ix].kind, Kind::Region(_) | Kind::Root));
        let carrying = chosen.map(|ix| site.arrows_of(ix, &shown));
        for arrow in arrows
            .iter()
            .filter(|arrow| {
                let (a, b) = (site.nodes[arrow.from].rect, site.nodes[arrow.to].rect);
                a.overlaps(&view) || b.overlaps(&view)
            })
            .take(MAX_ARROWS)
        {
            let (a, b) = (site.nodes[arrow.from].rect, site.nodes[arrow.to].rect);
            let (from, to) = edge_points(a, b);
            scene.arrows.push(Arrow {
                from,
                to,
                keys: (
                    site.nodes[arrow.from].key.clone(),
                    site.nodes[arrow.to].key.clone(),
                ),
                across: false,
                dashed: false,
                calls: Some(arrow.calls.clone()),
                alpha: alpha[arrow.from].min(alpha[arrow.to]).max(0.25),
                heads: (false, true),
                labels: Vec::new(),
                link: false,
                faint: false,
                picked: false,
                tint: None,
                title: None,
                title_tint: None,
                lit: carrying
                    .as_ref()
                    .map(|pairs| pairs.contains(&(arrow.from, arrow.to))),
            });
        }
        // Between regions, the domains' links, one per pair; what flows
        // along them only on the selected domain's.
        let picked = self.selected_region();
        let mut links: Vec<Arrow> = level
            .links
            .iter()
            .map(|link| {
                let mut arrow = picked_link(&level, link, picked.as_deref());
                if !arrow.picked {
                    arrow.labels.clear();
                }
                arrow
            })
            .collect();
        links.sort_by_key(|arrow| arrow.picked);
        scene.arrows.extend(links);
        scene
    }

    /// The arrows between the Site boxes `shown`, worked out again only
    /// when which boxes show changes.
    pub(super) fn site_arrows(&self, site: &Site, shown: &[usize]) -> super::super::SiteArrows {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        shown.hash(&mut hasher);
        (site as *const Site as usize).hash(&mut hasher);
        let key = hasher.finish();
        if let Some((at, arrows)) = self.site_arrows.borrow().as_ref()
            && *at == key
        {
            return arrows.clone();
        }
        let arrows = Rc::new(site.arrows(shown));
        *self.site_arrows.borrow_mut() = Some((key, arrows.clone()));
        arrows
    }
}
