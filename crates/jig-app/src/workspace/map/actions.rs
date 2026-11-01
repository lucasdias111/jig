//! The Map's actions started from the window: Index Domains, Update Domains and Explain.

use super::*;

impl Workspace {
    /// View > Index Domains or Update Domains: the Map at Domain, and the
    /// agent asked.
    pub(in crate::workspace) fn index_domains(
        &mut self,
        _: &IndexDomains,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.map_action_at_domain(false, window, cx);
    }

    /// View > Explain This Level: the agent asked about the Map's level,
    /// the Map shown first if it isn't.
    pub(in crate::workspace) fn explain_level(
        &mut self,
        _: &ExplainLevel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.map_showing().cloned() {
            return view.update(cx, |view, cx| view.explain_level(cx));
        }
        self.leave_tab(cx);
        let path = (!self.home).then(|| self.document().path.clone()).flatten();
        let offset = self.editor().cursor(cx);
        let Some(view) = self.open_map_pane(window, cx) else {
            return;
        };
        self.read_and(&view, cx, move |view, cx| {
            if let Some(path) = path {
                view.arrive_from_code(&path, offset, cx);
            }
            view.explain_level(cx);
        });
    }

    pub(in crate::workspace) fn update_domains(
        &mut self,
        _: &UpdateDomains,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.map_action_at_domain(true, window, cx);
    }

    fn map_action_at_domain(&mut self, update: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_tab(cx);
        let Some(view) = self.open_map_pane(window, cx) else {
            return;
        };
        self.read_and(&view, cx, move |view, cx| {
            view.go_to(Level::Domain, cx);
            if update {
                view.update_domains(cx);
            } else {
                view.start_action(Action::Index, cx);
            }
        });
    }
}
