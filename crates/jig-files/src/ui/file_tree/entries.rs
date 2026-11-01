//! What can be done to the selected rows: the right-click menu, trashing,
//! moving, copying their paths and revealing them.

use super::*;

impl FileTree {
    /// Right-clicking a row selects it and offers what can be done to it;
    /// right-clicking below the rows offers to make something at the root.
    pub(super) fn show_menu(
        &mut self,
        ix: Option<usize>,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_new(cx);
        // Right-clicking inside a selection of several acts on all of it.
        if ix.is_none_or(|ix| !self.is_selected(ix)) {
            self.marked.clear();
            self.selected = ix;
            self.anchor = self.selected_path();
        }
        // The menu's actions go to whatever has focus.
        self.focus_handle.focus(window, cx);
        cx.notify();
        let mut menu = NativeMenu::new()
            .menu("New File…", Box::new(NewFileHere))
            .menu("New Folder…", Box::new(NewFolderHere));
        if ix.is_some() {
            menu = menu.separator();
            if self.marked.is_empty() {
                menu = menu.menu("Rename…", Box::new(RenameSelected));
            }
            menu = menu
                .menu("Move to Trash", Box::new(TrashSelected))
                .separator()
                .menu("Copy Path", Box::new(CopyPath))
                .menu("Copy Relative Path", Box::new(CopyRelativePath));
        }
        menu.separator()
            .menu(reveal_label(), Box::new(RevealSelected))
            .show(position, window, cx);
    }

    pub(super) fn new_file_here(
        &mut self,
        _: &NewFileHere,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_new(NewEntry::File, window, cx);
    }

    pub(super) fn new_folder_here(
        &mut self,
        _: &NewFolderHere,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_new(NewEntry::Folder, window, cx);
    }

    pub(super) fn rename_selected(
        &mut self,
        _: &RenameSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_rename(window, cx);
    }

    /// Ask, then move the selected files and folders to the Trash. The
    /// Trash keeps them, so this can be undone from there.
    pub(super) fn trash_selected(
        &mut self,
        _: &TrashSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rows = self.selection();
        let named: Vec<(&str, bool)> = rows
            .iter()
            .map(|row| (row.name.as_str(), row.is_dir))
            .collect();
        let Some((title, detail)) = paths::trash_question(&named) else {
            return;
        };
        let paths = paths::outermost(&rows.iter().map(|row| row.path.clone()).collect::<Vec<_>>());
        let answer = window.prompt(
            PromptLevel::Warning,
            &title,
            Some(detail),
            &["Move to Trash", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await != Ok(0) {
                return;
            }
            this.update_in(cx, |this, window, cx| {
                let result = move_to_trash(&paths);
                this.model.refresh();
                this.marked.clear();
                let last = this.model.rows().len().checked_sub(1);
                this.selected = this.selected.zip(last).map(|(ix, last)| ix.min(last));
                this.anchor = this.selected_path();
                // A failure part way may have trashed some already.
                for path in paths.into_iter().filter(|path| !path.exists()) {
                    cx.emit(FileTreeEvent::Trashed(path));
                }
                if let Err(error) = result {
                    drop(window.prompt(
                        PromptLevel::Critical,
                        "Could not move everything to the Trash.",
                        Some(&error),
                        &["OK"],
                        cx,
                    ));
                }
                this.focus_handle.focus(window, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Move what's dragged into `dir`: the whole selection when the dragged
    /// row is part of it, else just that row. Open tabs follow.
    pub fn move_into(
        &mut self,
        dragged: &Path,
        dir: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_new(cx);
        let selection = self.selected_paths();
        let paths = if selection.iter().any(|path| path == dragged) {
            selection
        } else {
            vec![dragged.to_path_buf()]
        };
        let moved = move_entries(&paths, dir);
        self.model.refresh();
        if dir != self.model.root() {
            self.model.set_expanded(dir, true);
        }
        self.marked = moved.moves.iter().map(|(_, to)| to.clone()).collect();
        self.selected = moved
            .moves
            .first()
            .and_then(|(_, to)| self.model.index_of(to))
            .or_else(|| self.model.index_of(dragged));
        self.anchor = self.selected_path();
        self.forget_missing();
        self.focus_handle.focus(window, cx);
        for (from, to) in moved.moves {
            cx.emit(FileTreeEvent::Renamed { from, to });
        }
        if let Some(error) = moved.error {
            drop(window.prompt(PromptLevel::Critical, &error, None, &["OK"], cx));
        }
        cx.notify();
    }

    /// What a drag of `path` shows under the pointer.
    pub(super) fn drag_label(&self, path: &Path) -> SharedString {
        let selection = self.selection();
        if selection.len() > 1 && selection.iter().any(|row| row.path == path) {
            return format!("{} items", selection.len()).into();
        }
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
            .into()
    }

    /// Copy the selected paths, one per line.
    pub(super) fn copy_paths(&self, relative: bool, cx: &mut Context<Self>) {
        let root = self.model.root();
        let lines: Vec<String> = self
            .selected_paths()
            .iter()
            .map(|path| {
                let path = if relative {
                    path.strip_prefix(root).unwrap_or(path)
                } else {
                    path
                };
                path.display().to_string()
            })
            .collect();
        if !lines.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(lines.join("\n")));
        }
    }

    pub(super) fn copy_path(&mut self, _: &CopyPath, _: &mut Window, cx: &mut Context<Self>) {
        self.copy_paths(false, cx);
    }

    pub(super) fn copy_relative_path(
        &mut self,
        _: &CopyRelativePath,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.copy_paths(true, cx);
    }

    /// Show the selected entry, or the project folder, in the system's
    /// file manager.
    pub(super) fn reveal_selected(
        &mut self,
        _: &RevealSelected,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = self
            .selected_path()
            .unwrap_or_else(|| self.model.root().to_path_buf());
        cx.reveal_path(&path);
    }
}

pub(super) fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else if cfg!(target_os = "windows") {
        "Reveal in File Explorer"
    } else {
        "Reveal in File Manager"
    }
}
