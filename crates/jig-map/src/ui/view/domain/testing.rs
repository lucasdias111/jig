//! What jig-app's UI tests do at Domain.

use super::*;

impl MapView {
    /// Site close in on box `key`, the folder it's in open.
    pub fn show_site_box(&mut self, key: &str) {
        let ix = self
            .site
            .as_ref()
            .and_then(|s| s.index_of(key))
            .expect("a box");
        let to = self.camera_on_site_box(ix);
        self.set_camera(to);
    }

    pub fn domain_regions(&self) -> Vec<String> {
        self.domain_level()
            .map(|level| level.showing().map(|(_, r)| r.name.clone()).collect())
            .unwrap_or_default()
    }

    /// Where region `key` is at Domain.
    pub fn domain_rect(&self, key: &str) -> Option<Rect> {
        let level = self.domain_level()?;
        level
            .showing()
            .find(|(_, region)| region.key == key)
            .map(|(_, region)| region.rect)
    }

    pub fn has_proposal(&self) -> bool {
        self.domains.proposal.is_some()
    }

    pub fn banner(&self) -> Option<String> {
        self.domain_banner()
    }

    /// Where region `name` is in the window: its middle, or a point on its
    /// name, above it.
    pub fn region_point(&self, name: &str, middle: bool) -> Point<Pixels> {
        let level = self.domain_level().expect("domains");
        let region = level
            .regions
            .iter()
            .find(|region| region.name == name)
            .unwrap_or_else(|| panic!("no region {name}"));
        let at = if middle {
            region.rect.centre()
        } else {
            (
                region.rect.x + 30. / self.zoom,
                region.rect.y - 12. / self.zoom,
            )
        };
        self.bounds.get().origin + self.to_canvas(at)
    }

    pub fn domain_input(&self) -> Option<Entity<InputState>> {
        self.domains.editing.as_ref().map(|e| e.input.clone())
    }

    pub fn tint_at(&self, place: &str) -> Option<usize> {
        self.tint_of(place)
    }
}
