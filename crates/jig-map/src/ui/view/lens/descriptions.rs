//! Purposes and labels the agent described, kept or rejected.

use super::*;

impl MapView {
    /// The conversation's descriptions waiting to be kept or not.
    pub(in crate::ui::view) fn descriptions_waiting(&self) -> Option<Source> {
        self.lens
            .drawings
            .iter()
            .find(|(_, drawing)| drawing.kind == crate::domain::lens::Kind::Describe)
            .map(|(id, _)| Source::Conversation(*id))
    }

    /// The agent's descriptions in drawing `source` kept or not: kept, the
    /// purposes go where the Map keeps them and the drawing goes either
    /// way.
    pub fn settle_descriptions(&mut self, source: Source, keep: bool, cx: &mut Context<Self>) {
        let Some(drawing) = self.drawing(source).cloned() else {
            return;
        };
        if keep {
            self.keep_descriptions(&drawing, cx);
        }
        if let Source::Conversation(id) = source {
            cx.emit(MapEvent::Settled(id, keep));
        }
        cx.notify();
    }

    /// Keep the purposes and labels `drawing` describes: a domain's in
    /// `domains.toml`, with the label of a link between two domains; the
    /// rest in `.jig/map/purposes.toml`.
    pub(super) fn keep_descriptions(&mut self, drawing: &Drawing, cx: &mut Context<Self>) {
        let mut kept = purposes::read(&self.root);
        let mut domain_purposes = Vec::new();
        let mut flows = Vec::new();
        for step in &drawing.steps {
            match self.place(&step.place) {
                Some(Place::Domain(name)) => domain_purposes.push((name, step.label.clone())),
                Some(place) => {
                    if let Some(at) = place.site_place() {
                        kept.purposes.insert(at, step.label.clone());
                    }
                }
                None => {}
            }
        }
        for link in &drawing.links {
            match (self.place(&link.from), self.place(&link.to)) {
                (Some(Place::Domain(from)), Some(Place::Domain(to))) => {
                    flows.push((from, to, link.label.clone()))
                }
                (Some(from), Some(to)) => {
                    let name = |place: &Place| match place {
                        Place::Domain(name) => format!("domain:{name}"),
                        place => place.site_place().unwrap_or_default(),
                    };
                    kept.set_label(&name(&from), &name(&to), &link.label);
                }
                _ => {}
            }
        }
        if let Err(error) = purposes::write(&self.root, &kept) {
            self.say(&format!("{error:#}"), cx);
        }
        if !domain_purposes.is_empty() || !flows.is_empty() {
            self.edit_domains(
                |domains| {
                    for (name, purpose) in &domain_purposes {
                        if let Some(ix) = domains.index_of(name) {
                            domains.set_purpose(ix, purpose);
                        }
                    }
                    for (from, to, label) in &flows {
                        domains.set_flow(from, to, label);
                    }
                },
                cx,
            );
        } else {
            self.domains_changed();
        }
    }
}
