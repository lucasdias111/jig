//! Opening files in tabs, and following them as they're reloaded, renamed or trashed.

use super::*;

impl Workspace {
    pub(in crate::workspace) fn new_tab(
        &mut self,
        document: Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Tab {
        let editor = &self.settings.editor;
        if let Some(path) = document.path.as_deref() {
            self.file_watch.watch(path);
        }
        let detected = jig_kit::indentation::detect(&document.saved_text);
        let indentation = match document.path.as_deref() {
            Some(path) => jig_kit::editorconfig::EditorConfig::for_file(path).tab_size(detected),
            None => detected,
        };
        let state = cx.new(|cx| {
            EditorState::new(window, cx)
                // Jig's own find panel takes ⌘F, over the same search.
                .searchable(false)
                .language(document.language(&self.settings.languages))
                .tab_size(indentation)
                .line_number(editor.line_numbers)
                .soft_wrap(editor.soft_wrap)
                .indent_guides(editor.indent_guides)
                .show_whitespaces(editor.show_whitespace)
                .default_value(document.saved_text.clone())
        });
        super::super::definitions::install(&state, cx);
        let offers = super::super::completions::install(&state, cx);
        super::super::quick_fix::install(&state, cx);
        let events = cx.subscribe_in(
            &state,
            window,
            |this, state, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(ix) = this.tabs.iter().position(|tab| tab.editor.state() == state)
                {
                    this.refresh_dirty(ix, window, cx);
                    this.lsp_changed(ix, cx);
                    this.snippet_changed(ix, cx);
                    this.git_buffer_changed(ix, cx);
                    this.show_structure(ix, true, cx);
                    if ix == this.active {
                        this.on_buffer_changed(cx);
                    }
                }
                if matches!(event, InputEvent::PressEnter { .. })
                    && this.tab().editor.state() == state
                {
                    this.completions_after_enter(window, cx);
                }
                cx.notify();
            },
        );
        let focused = cx.on_focus(&state.focus_handle(cx), window, |this, _, cx| {
            this.code_focused(cx)
        });
        let cursor = cx.new(|cx| CursorPosition::new(state.clone(), cx));
        let marks = self.install_marks(&state, &document, cx);
        self.install_git(&state, &document, cx);
        let problems = super::super::diagnostics::Problems::new(
            &state,
            document.path.as_deref(),
            cx.entity().downgrade(),
            cx,
        );
        let hover = super::super::hover::install(&state, &problems.hover, cx);
        let structure = self.install_structure(&state, cx);
        let occurrences = super::super::occurrences::Occurrences::new(&state, cx);
        let lsp = document.path.as_deref().and_then(|path| {
            let language = document.language(&self.settings.languages);
            super::super::lsp::TabLsp::open(path, language, &document.saved_text, cx)
        });
        Tab {
            document,
            editor: KitEditor::new(state, cx),
            dirty: false,
            indentation,
            cursor,
            lsp,
            offers,
            snippet: None,
            marks,
            problems,
            hover,
            structure,
            occurrences,
            _events: events,
            _focused: focused,
        }
    }

    fn refresh_dirty(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tab = &mut self.tabs[ix];
        let dirty = tab.document.is_dirty(&tab.editor.text(cx));
        if dirty != tab.dirty {
            tab.dirty = dirty;
            self.update_title(window);
        }
    }

    pub(in crate::workspace) fn tab_for(&self, path: &Path) -> Option<usize> {
        let path = canonical(path);
        self.tabs.iter().position(|tab| {
            tab.document
                .path
                .as_deref()
                .is_some_and(|open| canonical(open) == path)
        })
    }

    /// Show `path`: switch to its tab if it is open, otherwise open it in a
    /// new tab next to the current one.
    pub fn open_file(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.tab_for(path) {
            self.activate(ix, window, cx);
            return;
        }
        let document = match Document::open(path) {
            Ok(document) => document,
            Err(error) => return self.show_error(&format!("{error:#}"), window, cx),
        };
        self.leave_home(cx);
        let tab = self.new_tab(document, window, cx);
        self.leave_tab(cx);
        if self.tab().is_blank(cx) {
            self.tabs[self.active] = tab;
        } else {
            self.active += 1;
            self.tabs.insert(self.active, tab);
        }
        self.enter_tab(window, cx);
    }

    /// Open `path` in a new last tab without switching to it, for edits
    /// made in files you aren't looking at. Returns its index.
    pub(in crate::workspace) fn open_in_background(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let document = Document::open(path).ok()?;
        let tab = self.new_tab(document, window, cx);
        self.tabs.push(tab);
        cx.notify();
        Some(self.tabs.len() - 1)
    }

    /// Re-open the current tab's file from disk, e.g. so highlighting follows
    /// a new extension after Save As.
    pub(in crate::workspace) fn reload_tab(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match Document::open(path) {
            Ok(document) => {
                self.remember_breakpoints(self.active, cx);
                let tab = self.new_tab(document, window, cx);
                self.leave_tab(cx);
                self.tabs[self.active] = tab;
                self.enter_tab(window, cx);
            }
            Err(error) => self.show_error(&format!("{error:#}"), window, cx),
        }
    }

    /// Point the tabs of files inside `from` at `to` after a rename in the
    /// file tree. Tabs without unsaved changes reopen from the new path, so
    /// highlighting follows a new extension; the others keep their edits
    /// and save to the new path.
    pub(in crate::workspace) fn follow_rename(
        &mut self,
        from: &Path,
        to: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for ix in 0..self.tabs.len() {
            let Some(path) = self.tabs[ix].document.path.as_deref() else {
                continue;
            };
            let Some(new) = jig_files::file_tree::moved_path(&canonical(path), from, to) else {
                continue;
            };
            if self.tabs[ix].dirty {
                self.tabs[ix].document.path = Some(new);
                continue;
            }
            let Ok(document) = Document::open(&new) else {
                continue;
            };
            if ix == self.active {
                self.leave_tab(cx);
            }
            self.remember_breakpoints(ix, cx);
            self.tabs[ix] = self.new_tab(document, window, cx);
        }
        self.update_title(window);
        if let Some(path) = self.document().path.clone() {
            self.show_in_tree(&path, window, cx);
        }
        self.remember_session(cx);
        cx.notify();
    }

    /// Close the tabs of files inside `path`, just moved to the Trash.
    /// Tabs with unsaved changes stay open so nothing typed is lost.
    pub(in crate::workspace) fn close_trashed(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ids: Vec<EntityId> = self
            .tabs
            .iter()
            .filter(|tab| {
                !tab.dirty
                    && tab
                        .document
                        .path
                        .as_deref()
                        .is_some_and(|open| canonical(open).starts_with(path))
            })
            .map(Tab::id)
            .collect();
        for id in ids {
            self.remove_tab(id, window, cx);
        }
    }
}
