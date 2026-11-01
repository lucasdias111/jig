//! Editing the domains by hand, saved at once, with undo and redo.

use super::*;

impl MapView {
    /// The domains now, kept for ⌘Z.
    pub(super) fn remember(&mut self) {
        self.domains.undo.push(Snapshot {
            saved: self.domains.saved.clone(),
            proposal: self.domains.proposal.clone(),
        });
        self.domains.redo.clear();
    }

    /// `saved` written to `domains.toml` (or the file removed, for none)
    /// and shown.
    pub(super) fn save_domains(&mut self, saved: Option<Domains>, cx: &mut Context<Self>) {
        let written = match &saved {
            Some(domains) => domains.write(&self.root),
            None => persist::domains::remove(&self.root),
        }
        .map_err(|e| format!("{e:#}"));
        if let Err(error) = written {
            self.say(&format!("Couldn't save the domains: {error}"), cx);
        }
        self.domains.saved = saved;
        persist::places::keep_out_of_git(&self.root);
        self.domains_changed();
        cx.notify();
    }

    /// An edit by hand: to the proposal while there is one, else to the
    /// saved domains, which are written at once. ⌘Z undoes it.
    pub(in crate::ui::view) fn edit_domains(
        &mut self,
        edit: impl FnOnce(&mut Domains),
        cx: &mut Context<Self>,
    ) {
        if let Some(proposal) = &self.domains.proposal {
            let mut domains = proposal.domains.clone();
            edit(&mut domains);
            if domains == proposal.domains {
                return;
            }
            self.remember();
            if let Some(proposal) = &mut self.domains.proposal {
                proposal.domains = domains;
            }
            self.domains_changed();
            cx.notify();
            return;
        }
        let Some(saved) = self.domains.saved.clone() else {
            return self.say(
                "These are Jig's drafts: Index Domains first, then edit what it finds.",
                cx,
            );
        };
        let mut domains = saved.clone();
        edit(&mut domains);
        if domains == saved {
            return;
        }
        self.remember();
        self.save_domains(Some(domains), cx);
    }

    /// ⌘Z: the domains as they were before the last edit.
    pub fn undo_domains(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(snapshot) = self.domains.undo.pop() else {
            return false;
        };
        self.domains.redo.push(Snapshot {
            saved: self.domains.saved.clone(),
            proposal: self.domains.proposal.clone(),
        });
        self.restore(snapshot, cx);
        true
    }

    /// ⇧⌘Z: the edit undone, again.
    pub fn redo_domains(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(snapshot) = self.domains.redo.pop() else {
            return false;
        };
        self.domains.undo.push(Snapshot {
            saved: self.domains.saved.clone(),
            proposal: self.domains.proposal.clone(),
        });
        self.restore(snapshot, cx);
        true
    }

    pub(super) fn restore(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        self.domains.proposal = snapshot.proposal;
        if snapshot.saved != self.domains.saved {
            self.save_domains(snapshot.saved, cx);
        } else {
            self.domains_changed();
            cx.notify();
        }
    }

    /// Edit domain `domain`'s name or purpose in place.
    pub fn begin_edit(
        &mut self,
        domain: usize,
        field: Field,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.drafts() {
            return self.say(
                "These are Jig's drafts: Index Domains first, then edit what it finds.",
                cx,
            );
        }
        let Some(shown) = self.shown_domains() else {
            return;
        };
        let Some(was) = shown.domains.get(domain) else {
            return;
        };
        let value = match field {
            Field::Name => was.name.clone(),
            Field::Purpose => was.purpose.clone(),
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
        let events = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::PressEnter { .. } => {
                this.finish_edit(true, cx);
                this.focus.focus(window, cx);
            }
            InputEvent::Blur => this.finish_edit(true, cx),
            _ => {}
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.domains.editing = Some(Editing {
            domain,
            field,
            input,
            _events: events,
        });
        cx.notify();
    }

    /// The edit in place done: kept, or given up.
    pub fn finish_edit(&mut self, keep: bool, cx: &mut Context<Self>) {
        let Some(editing) = self.domains.editing.take() else {
            return;
        };
        let text = editing.input.read(cx).value().to_string();
        if keep {
            let (domain, field) = (editing.domain, editing.field);
            self.edit_domains(
                |domains| {
                    if domain >= domains.domains.len() {
                        return;
                    }
                    match field {
                        Field::Name => domains.rename(domain, &text),
                        Field::Purpose => domains.set_purpose(domain, &text),
                    }
                },
                cx,
            );
            // The region's key is its name: keep it selected.
            if field == Field::Name
                && let Some(level) = self.domain_level()
                && let Some(region) = level
                    .regions
                    .iter()
                    .find(|region| region.kind == RegionKind::Domain(domain))
            {
                self.selected = vec![region.key.clone()];
            }
        }
        cx.notify();
    }

    /// Whether a field of the Map's has the keyboard.
    pub fn editing(&self, window: &Window, cx: &App) -> bool {
        self.domains
            .editing
            .as_ref()
            .is_some_and(|editing| editing.input.focus_handle(cx).is_focused(window))
            || self.editing_sketch(window, cx)
    }

    /// Where a dragged file or type would go if dropped at `position`: the
    /// home there, at Domain its region, at Site the domain of the box.
    pub(in crate::ui::view) fn drop_home(&self, position: Point<Pixels>) -> Option<(Home, String)> {
        match self.level {
            Level::Domain => {
                let level = self.domain_level()?;
                let ix = self.region_at(position)?;
                let region = &level.regions[ix];
                (region.kind != RegionKind::Unsorted).then(|| (region.home(), region.name.clone()))
            }
            Level::Site => {
                let saved = self.domains.saved.as_ref()?;
                let ix = self.site_box_at(position)?;
                let region = &self.site_nodes()[self.site_nodes()[ix].region];
                match region.kind {
                    Kind::Region(Home::Domain(domain)) => Some((
                        Home::Domain(domain),
                        saved.domains.get(domain)?.name.clone(),
                    )),
                    Kind::Region(Home::Shared) => Some((Home::Shared, "Shared".into())),
                    _ => None,
                }
            }
            Level::Plan(_) => None,
        }
    }

    /// The home `place` has in the domains showing.
    pub(in crate::ui::view) fn home_now(&self, place: &str) -> Home {
        match &self.domains.proposal {
            Some(proposal) => proposal.domains.home_of(place),
            None => self
                .domains
                .saved
                .as_ref()
                .map_or(Home::Unsorted, |saved| saved.home_of(place)),
        }
    }
}
