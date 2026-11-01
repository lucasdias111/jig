//! What jig-app's UI tests read of the Map.

use super::*;

impl MapView {
    pub fn plan(&self) -> &Plan {
        &self.plan
    }

    pub fn site(&self) -> Option<&Site> {
        self.site.as_deref()
    }

    pub fn zoom(&self) -> f32 {
        self.zoom
    }

    pub fn selected_keys(&self) -> &[String] {
        &self.selected
    }

    /// Whether a level is still cross-fading in.
    pub fn crossfading(&self) -> bool {
        self.transition.as_ref().is_some_and(Transition::fades)
    }

    /// The Plan box titled `title`.
    pub fn titled(&self, title: &str) -> usize {
        self.plan
            .nodes
            .iter()
            .position(|node| node.title == title)
            .unwrap_or_else(|| {
                panic!(
                    "no {title} in {:?}",
                    self.plan.nodes.iter().map(|n| &n.title).collect::<Vec<_>>()
                )
            })
    }

    /// Where Plan box `ix`'s header is in the window, for tests to press.
    pub fn node_point(&self, ix: usize) -> Point<Pixels> {
        let rect = self.placed.nodes[ix];
        self.bounds.get().origin
            + self.to_canvas((
                rect.x + rect.width / 2.,
                rect.y + layout::HEADER_HEIGHT / 2.,
            ))
    }

    /// Where Site box `key` is in the window, its middle.
    pub fn site_point(&self, key: &str) -> Point<Pixels> {
        let site = self.site.as_ref().expect("read");
        let rect = site.nodes[site.index_of(key).expect("a box")].rect;
        self.bounds.get().origin + self.to_canvas(rect.centre())
    }

    /// The Site boxes showing, by key, and whether each is open.
    pub fn site_showing(&self) -> Vec<(String, bool)> {
        self.drawn
            .borrow()
            .frames
            .iter()
            .map(|frame| (frame.key.clone(), true))
            .chain(
                self.drawn
                    .borrow()
                    .cards
                    .iter()
                    .map(|card| (card.key.clone(), false)),
            )
            .collect()
    }

    /// The words on the arrows drawn: flows, and calls leaving a region.
    pub fn drawn_labels(&self) -> Vec<String> {
        self.drawn
            .borrow()
            .arrows
            .iter()
            .flat_map(|arrow| arrow.labels.iter().map(|(_, label)| label.clone()))
            .collect()
    }

    /// Whether each drawn link's words are faint.
    pub fn drawn_faint(&self) -> Vec<bool> {
        self.drawn
            .borrow()
            .arrows
            .iter()
            .filter(|a| a.link)
            .map(|a| a.faint)
            .collect()
    }

    /// How many links between domains are drawn.
    pub fn drawn_links(&self) -> usize {
        self.drawn.borrow().arrows.iter().filter(|a| a.link).count()
    }

    /// The arrows drawn at Site between boxes, by their ends' keys, and
    /// whether each is lit for the box selected (`None`: nothing is).
    pub fn site_arrows_lit(&self) -> Vec<((String, String), Option<bool>)> {
        self.drawn
            .borrow()
            .arrows
            .iter()
            .filter(|arrow| !arrow.link)
            .map(|arrow| (arrow.keys.clone(), arrow.lit))
            .collect()
    }

    /// The regions drawn lit, by key.
    pub fn lit_regions(&self) -> Vec<String> {
        self.drawn
            .borrow()
            .regions
            .iter()
            .filter(|region| region.lit)
            .map(|region| region.key.clone())
            .collect()
    }

    /// The headings of the selected domain's links drawn: the domains at
    /// their other ends.
    pub fn picked_links(&self) -> Vec<String> {
        self.drawn
            .borrow()
            .arrows
            .iter()
            .filter(|arrow| arrow.picked)
            .filter_map(|arrow| arrow.title.clone())
            .collect()
    }

    /// The regions drawn, by key, and how far each is open.
    pub fn drawn_regions(&self) -> Vec<(String, Rect, f32)> {
        self.drawn
            .borrow()
            .regions
            .iter()
            .map(|region| (region.key.clone(), region.rect, region.open))
            .collect()
    }

    /// Where box `key` of Site is drawn in the window, if it's in view.
    pub fn site_box_on_screen(&self, key: &str) -> Option<Rect> {
        let site = self.site.as_ref()?;
        let rect = site.nodes[site.index_of(key)?].rect;
        let at = self.to_canvas((rect.x, rect.y));
        Some(Rect {
            x: f32::from(at.x),
            y: f32::from(at.y),
            width: rect.width * self.zoom,
            height: rect.height * self.zoom,
        })
    }

    /// The Plan's functions lit as the debugger's path.
    pub fn lit_path(&self) -> Vec<String> {
        let project = self.project.as_ref().expect("read");
        self.on_stack
            .iter()
            .map(|&id| project.functions[id].title())
            .collect()
    }

    pub fn firing_of(&self, title: &str) -> f32 {
        let key = &self.plan.nodes[self.titled(title)].key;
        self.firing.of_box(key, Instant::now())
    }
}
