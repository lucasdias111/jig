//! Which rows are selected, and moving the selection with the keyboard
//! and the pointer.

use super::*;

impl FileTree {
    /// The selected rows, top to bottom.
    pub(super) fn selection(&self) -> Vec<Row> {
        if self.marked.is_empty() {
            return self
                .selected
                .and_then(|ix| self.model.rows().get(ix))
                .cloned()
                .into_iter()
                .collect();
        }
        self.model
            .rows()
            .iter()
            .filter(|row| self.marked.contains(&row.path))
            .cloned()
            .collect()
    }

    pub(super) fn selected_paths(&self) -> Vec<PathBuf> {
        self.selection().into_iter().map(|row| row.path).collect()
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn selected_names(&self) -> Vec<String> {
        self.selection().into_iter().map(|row| row.name).collect()
    }

    pub(super) fn is_selected(&self, ix: usize) -> bool {
        self.selected == Some(ix)
            || self
                .model
                .rows()
                .get(ix)
                .is_some_and(|row| self.marked.contains(&row.path))
    }

    pub(super) fn selected_path(&self) -> Option<PathBuf> {
        self.selected
            .and_then(|ix| self.model.rows().get(ix))
            .map(|row| row.path.clone())
    }

    /// Select just row `ix`.
    pub(super) fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.marked.clear();
        self.selected = Some(ix);
        self.anchor = self.selected_path();
        self.scroll_handle
            .scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// ⌘-click: add row `ix` to the selection, or take it out.
    pub(super) fn toggle_selected(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(path) = self.model.rows().get(ix).map(|row| row.path.clone()) else {
            return;
        };
        if self.marked.is_empty() {
            self.marked.extend(self.selected_path());
        }
        if self.marked.remove(&path) {
            self.selected = self
                .model
                .rows()
                .iter()
                .position(|row| self.marked.contains(&row.path));
        } else {
            self.marked.insert(path.clone());
            self.selected = Some(ix);
        }
        self.anchor = self.selected_path();
        if self.marked.len() < 2 {
            self.marked.clear();
        }
        cx.notify();
    }

    /// Shift-click or Shift-arrow: select every row from the anchor to `ix`.
    pub(super) fn extend_to(&mut self, ix: usize, cx: &mut Context<Self>) {
        let from = self
            .anchor
            .as_deref()
            .and_then(|path| self.model.index_of(path))
            .or(self.selected)
            .unwrap_or(ix);
        let range = from.min(ix)..=from.max(ix);
        self.marked = self.model.rows()[range]
            .iter()
            .map(|row| row.path.clone())
            .collect();
        if self.marked.len() < 2 {
            self.marked.clear();
        }
        if self.anchor.is_none() {
            self.anchor = self.model.rows().get(from).map(|row| row.path.clone());
        }
        self.selected = Some(ix);
        self.scroll_handle
            .scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    pub(super) fn extend_previous(
        &mut self,
        _: &ExtendPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ix = self.selected.map_or(0, |ix| ix.saturating_sub(1));
        self.extend_to(ix, cx);
    }

    pub(super) fn extend_next(&mut self, _: &ExtendNext, _: &mut Window, cx: &mut Context<Self>) {
        let Some(last) = self.model.rows().len().checked_sub(1) else {
            return;
        };
        let ix = self.selected.map_or(0, |ix| (ix + 1).min(last));
        self.extend_to(ix, cx);
    }

    pub(super) fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = self
            .model
            .rows()
            .iter()
            .map(|row| row.path.clone())
            .collect();
        if self.marked.len() < 2 {
            self.marked.clear();
        }
        if self.selected.is_none() && !self.model.rows().is_empty() {
            self.selected = Some(0);
        }
        cx.notify();
    }

    /// A press on row `ix`, before it's known to be a click or a drag. A
    /// plain press inside a selection of several keeps it, so it can be
    /// dragged; the click that follows, if it's one, selects just the row.
    pub(super) fn press_row(&mut self, ix: usize, modifiers: Modifiers, cx: &mut Context<Self>) {
        if modifiers.secondary() {
            self.toggle_selected(ix, cx);
        } else if modifiers.shift {
            self.extend_to(ix, cx);
        } else if !self.marked.is_empty() && self.is_selected(ix) {
            self.selected = Some(ix);
            cx.notify();
        } else {
            self.select(ix, cx);
        }
    }

    pub(super) fn click_row(
        &mut self,
        ix: usize,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_handle.focus(window, cx);
        let modifiers = match event {
            ClickEvent::Mouse(click) => click.down.modifiers,
            _ => event.modifiers(),
        };
        if modifiers.secondary() || modifiers.shift {
            return;
        }
        self.select(ix, cx);
        self.activate(ix, cx);
    }

    /// Open a file, or expand or collapse a folder.
    pub(super) fn activate(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.model.rows().get(ix).cloned() else {
            return;
        };
        self.selected = Some(ix);
        if row.is_dir {
            self.model.toggle(&row.path);
        } else {
            cx.emit(FileTreeEvent::Open(row.path));
        }
        cx.notify();
    }

    pub(super) fn select_previous(
        &mut self,
        _: &SelectPrevious,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ix = self.selected.map_or(0, |ix| ix.saturating_sub(1));
        self.select(ix, cx);
    }

    pub(super) fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        let last = self.model.rows().len().saturating_sub(1);
        let ix = self.selected.map_or(0, |ix| (ix + 1).min(last));
        self.select(ix, cx);
    }

    pub(super) fn collapse_or_parent(
        &mut self,
        _: &CollapseOrParent,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(row) = self
            .selected
            .and_then(|ix| self.model.rows().get(ix))
            .cloned()
        else {
            return;
        };
        if row.expanded {
            self.model.set_expanded(&row.path, false);
            cx.notify();
        } else if let Some(parent) = row.path.parent()
            && let Some(ix) = self.model.index_of(parent)
        {
            self.select(ix, cx);
        }
    }

    pub(super) fn expand_or_child(
        &mut self,
        _: &ExpandOrChild,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.selected else { return };
        let Some(row) = self.model.rows().get(ix).cloned() else {
            return;
        };
        if !row.is_dir {
            return;
        }
        if !row.expanded {
            self.model.set_expanded(&row.path, true);
            cx.notify();
        } else if self
            .model
            .rows()
            .get(ix + 1)
            .is_some_and(|next| next.depth > row.depth)
        {
            self.select(ix + 1, cx);
        }
    }

    pub(super) fn open_selected(
        &mut self,
        _: &OpenSelected,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(ix) = self.selected {
            self.activate(ix, cx);
        }
    }

    /// Escape: back to one selected row, then back to the code.
    pub(super) fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        if !self.marked.is_empty() {
            self.marked.clear();
            self.anchor = self.selected_path();
            cx.notify();
            return;
        }
        cx.emit(FileTreeEvent::Dismissed);
    }
}
