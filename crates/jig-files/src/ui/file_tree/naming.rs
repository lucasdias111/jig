//! The name field for making or renaming a file or folder in place.

use super::*;

impl FileTree {
    /// Where a new entry goes: the selected folder, the selected file's
    /// folder, or the root.
    pub(super) fn target_dir(&self) -> PathBuf {
        match self.selected.and_then(|ix| self.model.rows().get(ix)) {
            Some(row) if row.is_dir => row.path.clone(),
            Some(row) => row
                .path
                .parent()
                .map_or_else(|| self.model.root().to_path_buf(), Path::to_path_buf),
            None => self.model.root().to_path_buf(),
        }
    }

    /// Show a name field for a new file or folder in the target folder.
    pub fn start_new(&mut self, kind: NewEntry, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self.target_dir();
        if dir != self.model.root() {
            self.model.set_expanded(&dir, true);
        }
        let placeholder = match kind {
            NewEntry::File => "File name",
            NewEntry::Folder => "Folder name",
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        self.start_naming(dir, NamingFor::New(kind), input, window, cx);
    }

    /// Turn the selected row into a name field to rename it.
    pub(super) fn start_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.marked.clear();
        let Some(row) = self
            .selected
            .and_then(|ix| self.model.rows().get(ix))
            .cloned()
        else {
            return;
        };
        let Some(dir) = row.path.parent().map(Path::to_path_buf) else {
            return;
        };
        let stem = stem_range(&row.name, row.is_dir);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(row.name.clone()));
        self.start_naming(dir, NamingFor::Rename(row.path), input.clone(), window, cx);
        input.update(cx, |input, cx| input.set_selected_range(stem, cx));
    }

    pub(super) fn start_naming(
        &mut self,
        dir: PathBuf,
        purpose: NamingFor,
        input: Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let events = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::PressEnter { .. } => this.finish_naming(window, cx),
            InputEvent::Blur => this.cancel_new(cx),
            InputEvent::Change => {
                if let Some(naming) = &mut this.naming {
                    naming.error = None;
                }
                cx.notify();
            }
            InputEvent::Focus => {}
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        self.naming = Some(Naming {
            dir,
            purpose,
            input,
            error: None,
            _events: events,
        });
        if let Some(ix) = self.naming_index() {
            self.scroll_handle
                .scroll_to_item(ix, ScrollStrategy::Nearest);
        }
        cx.notify();
    }

    pub(super) fn finish_naming(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(naming) = &mut self.naming else {
            return;
        };
        let name = naming.input.read(cx).value().to_string();
        let done = match &naming.purpose {
            NamingFor::New(kind) => create_entry(&naming.dir, &name, *kind).map(|path| {
                let open = (*kind == NewEntry::File).then(|| FileTreeEvent::Open(path.clone()));
                (path, open)
            }),
            NamingFor::Rename(from) => rename_entry(from, &name).map(|to| {
                let renamed = (to != *from).then(|| FileTreeEvent::Renamed {
                    from: from.clone(),
                    to: to.clone(),
                });
                (to, renamed)
            }),
        };
        match done {
            Ok((path, event)) => {
                self.naming = None;
                self.model.refresh();
                self.selected = self.model.reveal(&path);
                self.focus_handle.focus(window, cx);
                if let Some(event) = event {
                    cx.emit(event);
                }
            }
            Err(error) => naming.error = Some(error),
        }
        cx.notify();
    }

    pub(super) fn cancel_new(&mut self, cx: &mut Context<Self>) {
        if self.naming.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn on_naming_escape(
        &mut self,
        _: &Escape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.naming.is_none() {
            cx.propagate();
            return;
        }
        self.cancel_naming(&CancelNaming, window, cx);
        cx.stop_propagation();
    }

    pub(super) fn cancel_naming(
        &mut self,
        _: &CancelNaming,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_new(cx);
        self.focus_handle.focus(window, cx);
    }

    /// The list position of the name field: first in its folder for a new
    /// entry, in place of the row it renames otherwise.
    pub(super) fn naming_index(&self) -> Option<usize> {
        let naming = self.naming.as_ref()?;
        if let NamingFor::Rename(path) = &naming.purpose {
            return self.model.index_of(path);
        }
        if naming.dir == self.model.root() {
            return Some(0);
        }
        self.model.index_of(&naming.dir).map(|ix| ix + 1)
    }

    /// Whether the name field is a row of its own rather than in place of one.
    pub(super) fn naming_inserts(&self) -> bool {
        self.naming
            .as_ref()
            .is_some_and(|naming| matches!(naming.purpose, NamingFor::New(_)))
    }

    pub(super) fn naming_depth(&self) -> usize {
        let Some(naming) = &self.naming else { return 0 };
        self.model
            .index_of(&naming.dir)
            .map_or(0, |ix| self.model.rows()[ix].depth + 1)
    }
}
