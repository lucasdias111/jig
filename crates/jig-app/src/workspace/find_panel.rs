//! Find (⌘F) and Find and Replace (⌥⌘F) in the current file: opening the
//! panel over the code and closing it again.

use gpui_kit::*;
use jig_editor::EditorHandle;

use super::{Find, FindAndReplace, Workspace, find};
use crate::find_panel::{FindPanel, FindPanelEvent};

pub(super) struct OpenFindPanel {
    pub(super) view: Entity<FindPanel>,
    _events: Subscription,
}

impl Workspace {
    pub(super) fn find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find_panel(false, window, cx);
    }

    pub(super) fn find_and_replace(
        &mut self,
        _: &FindAndReplace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_find_panel(true, window, cx);
    }

    /// Open the panel, or go back to it, starting from a short one-line
    /// selection when there is one. Not over the ⌘K palette or another
    /// panel: they have the keyboard.
    fn open_find_panel(&mut self, replacing: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.home || self.modal_open() {
            return;
        }
        let selection = self.editor().selection(cx);
        let selected = find::selection_query(&self.editor().text(cx), selection.clone());
        if let Some(open) = &self.find_panel {
            // Typing in the panel moves the selection to each match, so
            // only a selection made in the code is a new query.
            let selected = selected.filter(|_| !open.view.read(cx).has_focus(window, cx));
            open.view.update(cx, |panel, cx| {
                panel.reopen(selected.as_deref(), replacing, window, cx)
            });
            return;
        }
        let query = selected.unwrap_or_else(|| self.last_find_query.clone());
        let state = self.editor().state().clone();
        let view =
            cx.new(|cx| FindPanel::new(state, &query, selection.start, replacing, window, cx));
        let events = cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
            FindPanelEvent::Dismissed => {
                this.close_find_panel(cx);
                this.editor().focus(window, cx);
            }
        });
        self.find_panel = Some(OpenFindPanel {
            view,
            _events: events,
        });
        cx.notify();
    }

    /// Close the panel and drop its highlights. The query is kept for the
    /// next ⌘F. Returns whether it was open.
    pub(super) fn close_find_panel(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(open) = self.find_panel.take() else {
            return false;
        };
        self.last_find_query = open.view.read(cx).query(cx);
        // The panel searched the tab it opened on, which may not be the
        // active one any more.
        for tab in &self.tabs {
            tab.editor
                .state()
                .update(cx, |state, cx| state.close_search(cx));
        }
        cx.notify();
        true
    }

    /// Whether the keyboard is in the find panel, so that Enter, Tab, Esc
    /// and ⌘Z are its, not the review's.
    pub(super) fn find_panel_focused(&self, window: &Window, cx: &App) -> bool {
        self.find_panel
            .as_ref()
            .is_some_and(|open| open.view.read(cx).has_focus(window, cx))
    }

    /// The panel, at the top right of the code.
    pub(super) fn render_find_panel(&self) -> Option<AnyElement> {
        let open = self.find_panel.as_ref()?;
        Some(
            div()
                .absolute()
                .top_2()
                .right_4()
                .child(open.view.clone())
                .into_any_element(),
        )
    }
}
