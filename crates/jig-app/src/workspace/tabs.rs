//! Tabs: one open file each, with its own buffer, unsaved state and undo
//! history, and the strip that shows them.

use std::path::Path;

use gpui_kit::component::input::{EditorState, InputEvent, TabSize};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_editor::{EditorHandle, KitEditor};

use super::status_bar::CursorPosition;
use super::{ActivateTab, CloseTab, NewFile, NextTab, PreviousTab, Workspace};
use jig_editing::document::Document;

mod opening;
mod tab_bar;

const CLOSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"#;

/// Tabs share the title bar's width between these bounds; past the
/// narrowest, the strip scrolls.
/// Height of the row of tabs under the title bar.
const TAB_BAR_HEIGHT: f32 = 36.;
const MIN_TAB_WIDTH: f32 = 120.;
const MAX_TAB_WIDTH: f32 = 260.;

pub(super) struct Tab {
    pub(super) document: Document,
    pub(super) editor: KitEditor,
    pub(super) dirty: bool,
    /// Detected from the file when it was opened, or as its
    /// `.editorconfig` says; what Tab inserts.
    pub(super) indentation: TabSize,
    pub(super) cursor: Entity<CursorPosition>,
    /// The file open in its language server, if it has one.
    pub(super) lsp: Option<super::lsp::TabLsp>,
    /// Snippets just offered in the completion list.
    pub(super) offers: super::snippets::Offers,
    /// The snippet being filled in, if any.
    pub(super) snippet: Option<super::snippets::Session>,
    /// Breakpoints, and the line the debugger is paused on.
    pub(super) marks: super::breakpoints::Marks,
    /// What the language server reports about the file.
    pub(super) problems: super::diagnostics::Problems,
    /// What the language server says about the name under the pointer and
    /// the call being typed.
    pub(super) hover: Entity<super::hover::CodeHover>,
    /// The brackets at the cursor and the guide of its block.
    pub(super) structure: super::brackets::Structure,
    /// The other uses of the name at the cursor.
    pub(super) occurrences: super::occurrences::Occurrences,
    _events: Subscription,
    /// The code getting focus, which shows it in place of the Map.
    _focused: Subscription,
}

impl Tab {
    /// Identifies the tab across reordering, e.g. while a prompt is open.
    pub(super) fn id(&self) -> EntityId {
        self.editor.state().entity_id()
    }

    /// An Untitled tab nobody has typed into, which opening a file reuses.
    pub(super) fn is_blank(&self, cx: &App) -> bool {
        self.document.path.is_none() && !self.dirty && self.editor.text(cx).is_empty()
    }
}

pub(super) fn canonical(path: &Path) -> std::path::PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

impl Workspace {
    pub(super) fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub(super) fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub(super) fn editor(&self) -> &KitEditor {
        &self.tab().editor
    }

    pub(super) fn document(&self) -> &Document {
        &self.tab().document
    }

    pub(super) fn any_dirty(&self) -> bool {
        self.tabs.iter().any(|tab| tab.dirty)
    }

    pub(super) fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        if ix != self.active {
            self.leave_tab(cx);
            self.active = ix;
        }
        self.enter_tab(window, cx);
    }

    /// Clear anything floating over the current tab before it goes out of
    /// view. The agent conversation belongs to the project, not the tab: it stays,
    /// working, and an edit it proposed waits in its file's tab.
    pub(super) fn leave_tab(&mut self, cx: &mut Context<Self>) {
        self.palette = None;
        self.close_find_panel(cx);
        self.note = None;
    }

    fn enter_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A tab is code: it shows in place of the Map.
        self.code_focused(cx);
        self.focus_code(window, cx);
        self.tab_scroll.scroll_to_item(self.active);
        self.update_title(window);
        if let Some(path) = self.document().path.clone() {
            self.note_recent_file(&path, cx);
            self.show_in_tree(&path, window, cx);
        }
        self.remember_session(cx);
        cx.notify();
    }

    pub(super) fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        if self.home {
            // The blank tab behind the home page is the new file.
            self.leave_home(cx);
            self.enter_tab(window, cx);
            return;
        }
        let tab = self.new_tab(Document::default(), window, cx);
        self.leave_tab(cx);
        self.active += 1;
        self.tabs.insert(self.active, tab);
        self.enter_tab(window, cx);
    }

    pub(super) fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab_at(self.active, window, cx);
    }

    /// Close the tab at `ix`, asking first if it has unsaved changes.
    /// Closing the last tab shows the start page; closing that closes the
    /// window.
    fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        if self.home {
            window.remove_window();
            return;
        }
        let id = tab.id();
        self.when_discard_ok(Some(&[id]), window, cx, move |this, window, cx| {
            this.remove_tab(id, window, cx)
        });
    }

    /// Close every tab, or with `keep`, every tab but that one, asking once
    /// first if any of them has unsaved changes.
    pub(super) fn close_tabs_except(
        &mut self,
        keep: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keep = keep.and_then(|ix| self.tabs.get(ix)).map(Tab::id);
        let ids: Vec<EntityId> = self
            .tabs
            .iter()
            .map(Tab::id)
            .filter(|id| Some(*id) != keep)
            .collect();
        if ids.is_empty() {
            return;
        }
        self.when_discard_ok(Some(&ids.clone()), window, cx, move |this, window, cx| {
            for id in ids {
                this.remove_tab(id, window, cx);
            }
            if let Some(ix) = keep.and_then(|id| this.tabs.iter().position(|tab| tab.id() == id)) {
                this.activate(ix, window, cx);
            }
        });
    }

    pub(super) fn remove_tab(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.tabs.iter().position(|tab| tab.id() == id) else {
            return;
        };
        if ix == self.active {
            self.leave_tab(cx);
        }
        self.remember_breakpoints(ix, cx);
        self.tabs.remove(ix);
        if self.tabs.is_empty() {
            let tab = self.new_tab(Document::default(), window, cx);
            self.tabs.push(tab);
            self.home = true;
            if self.tree.is_some() {
                self.focus_tree(window, cx);
            } else {
                self.focus_main(window, cx);
            }
            self.update_title(window);
            self.remember_session(cx);
            cx.notify();
            return;
        }
        // The tab to the right takes the closed one's place, or the one to
        // the left when it was last.
        if ix < self.active || self.active >= self.tabs.len() {
            self.active -= 1;
        }
        self.enter_tab(window, cx);
    }

    pub(super) fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.activate((self.active + 1) % self.tabs.len(), window, cx);
    }

    pub(super) fn previous_tab(
        &mut self,
        _: &PreviousTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let len = self.tabs.len();
        self.activate((self.active + len - 1) % len, window, cx);
    }

    /// Cmd+1…8 pick that tab; Cmd+9 always picks the last, as in browsers.
    pub(super) fn activate_tab(
        &mut self,
        action: &ActivateTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ix = if action.0 == 8 {
            self.tabs.len() - 1
        } else {
            action.0
        };
        self.activate(ix, window, cx);
    }
}
