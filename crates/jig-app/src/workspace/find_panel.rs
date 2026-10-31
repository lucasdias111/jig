//! Find (⌘F) and Find and Replace (⌥⌘F) in the current file: opening the
//! panel over the code and closing it again.

use gpui_kit::component::input::EditorState;
use gpui_kit::*;
use jig_editor::{EditorHandle, KitEditor};

use super::{Find, FindAndReplace, Workspace, find};
use jig_editing::find_panel::{FindPanel, FindPanelEvent};

/// The panel, while it's open, and the query it last had. Its own entity,
/// so the workspace only opens it, places it and closes it.
#[derive(Default)]
pub(super) struct FindInFile {
    open: Option<OpenFindPanel>,
    /// The panel's last query, to start the next ⌘F from.
    last_query: String,
}

struct OpenFindPanel {
    view: Entity<FindPanel>,
    /// The code it searches: the tab it opened on, which may not be the
    /// active one any more.
    searching: Entity<EditorState>,
    _events: Subscription,
}

/// The panel was dismissed from inside it, so the code gets the keyboard
/// back.
pub(super) struct FindDismissed;

impl EventEmitter<FindDismissed> for FindInFile {}

impl FindInFile {
    /// The panel, while it's open.
    pub(super) fn panel(&self) -> Option<&Entity<FindPanel>> {
        self.open.as_ref().map(|open| &open.view)
    }

    /// Whether the keyboard is in the panel.
    pub(super) fn has_focus(&self, window: &Window, cx: &App) -> bool {
        self.panel()
            .is_some_and(|view| view.read(cx).has_focus(window, cx))
    }

    /// Open the panel on `editor`, or go back to it, starting from a short
    /// one-line selection when there is one.
    fn open(
        &mut self,
        editor: &KitEditor,
        replacing: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let selection = editor.selection(cx);
        let selected = find::selection_query(&editor.text(cx), selection.clone());
        if let Some(open) = &self.open {
            // Typing in the panel moves the selection to each match, so
            // only a selection made in the code is a new query.
            let selected = selected.filter(|_| !open.view.read(cx).has_focus(window, cx));
            open.view.update(cx, |panel, cx| {
                panel.reopen(selected.as_deref(), replacing, window, cx)
            });
            return;
        }
        let query = selected.unwrap_or_else(|| self.last_query.clone());
        let state = editor.state().clone();
        let view = cx.new(|cx| {
            FindPanel::new(
                state.clone(),
                &query,
                selection.start,
                replacing,
                window,
                cx,
            )
        });
        let events = cx.subscribe(&view, |this, _, event, cx| match event {
            FindPanelEvent::Dismissed => {
                this.close(cx);
                cx.emit(FindDismissed);
            }
        });
        self.open = Some(OpenFindPanel {
            view,
            searching: state,
            _events: events,
        });
        cx.notify();
    }

    /// Close the panel and drop its highlights. The query is kept for the
    /// next ⌘F. Returns whether it was open.
    fn close(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(open) = self.open.take() else {
            return false;
        };
        self.last_query = open.view.read(cx).query(cx);
        open.searching
            .update(cx, |state, cx| state.close_search(cx));
        cx.notify();
        true
    }
}

impl Render for FindInFile {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        match &self.open {
            Some(open) => open.view.clone().into_any_element(),
            None => div().into_any_element(),
        }
    }
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

    /// Open the panel, or go back to it. Not over the ⌘K palette or
    /// another panel: they have the keyboard.
    fn open_find_panel(&mut self, replacing: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.home || self.modal_open(cx) {
            return;
        }
        let editor = self.editor().clone();
        self.find_panel
            .update(cx, |find, cx| find.open(&editor, replacing, window, cx));
        cx.notify();
    }

    /// The panel was dismissed from inside it: back to the code.
    pub(super) fn on_find_dismissed(
        &mut self,
        _: &Entity<FindInFile>,
        _: &FindDismissed,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.notify();
        self.focus_editor(window, cx);
    }

    /// Close the panel and drop its highlights. Returns whether it was
    /// open.
    pub(super) fn close_find_panel(&mut self, cx: &mut Context<Self>) -> bool {
        let closed = self.find_panel.update(cx, |find, cx| find.close(cx));
        if closed {
            cx.notify();
        }
        closed
    }

    /// Whether the keyboard is in the find panel, so that Enter, Tab, Esc
    /// and ⌘Z are its, not the review's.
    pub(super) fn find_panel_focused(&self, window: &Window, cx: &App) -> bool {
        self.find_panel.read(cx).has_focus(window, cx)
    }

    /// The panel, at the top right of the code, while it's open.
    pub(super) fn render_find_panel(&self, cx: &App) -> Option<AnyElement> {
        self.find_panel.read(cx).panel()?;
        Some(
            div()
                .absolute()
                .top_2()
                .right_4()
                .child(self.find_panel.clone())
                .into_any_element(),
        )
    }
}
