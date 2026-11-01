//! The project sidebar: every folder and file under the project root, read
//! lazily as folders are expanded.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::domain::paths::{self, NewEntry, stem_range};
use crate::store::cache::{Row, TreeModel};
use crate::store::persist::{create_entry, move_entries, move_to_trash, rename_entry};

mod entries;
mod keys;
mod naming;
mod selection;
mod view;

pub use keys::key_bindings;
use keys::*;

pub enum FileTreeEvent {
    /// A file was chosen.
    Open(PathBuf),
    /// Escape: hand focus back to the editor.
    Dismissed,
    /// A file or folder was renamed; tabs inside it should follow.
    Renamed { from: PathBuf, to: PathBuf },
    /// A file or folder was moved to the Trash; its tabs can close.
    Trashed(PathBuf),
}

impl EventEmitter<FileTreeEvent> for FileTree {}

/// What the name field is for.
#[derive(Clone, Debug, PartialEq)]
enum NamingFor {
    New(NewEntry),
    /// Renaming this entry; the field takes its row's place.
    Rename(PathBuf),
}

/// The name field shown in the tree while making or renaming a file or
/// folder.
struct Naming {
    /// The folder the entry goes in.
    dir: PathBuf,
    purpose: NamingFor,
    input: Entity<InputState>,
    error: Option<String>,
    _events: Subscription,
}

/// The sidebar view.
pub struct FileTree {
    model: TreeModel,
    naming: Option<Naming>,
    /// The row the keyboard is on: the selection, or the last row added to
    /// a selection of several.
    selected: Option<usize>,
    /// Every selected row while there are several, picked with ⌘- or
    /// Shift-click, Shift-arrows or ⌘A; empty while there's one.
    marked: HashSet<PathBuf>,
    /// Where a Shift range starts.
    anchor: Option<PathBuf>,
    /// The file open in the editor, highlighted in the tree.
    active: Option<PathBuf>,
    focus_handle: FocusHandle,
    scroll_handle: UniformListScrollHandle,
}

impl FileTree {
    pub fn new(root: &Path, cx: &mut Context<Self>) -> Self {
        Self {
            model: TreeModel::new(root),
            naming: None,
            selected: None,
            marked: HashSet::new(),
            anchor: None,
            active: None,
            focus_handle: cx.focus_handle(),
            scroll_handle: UniformListScrollHandle::default(),
        }
    }

    pub fn root(&self) -> &Path {
        self.model.root()
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn naming_error(&self) -> Option<Option<String>> {
        self.naming.as_ref().map(|naming| naming.error.clone())
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn model(&self) -> &TreeModel {
        &self.model
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_none() && !self.model.rows().is_empty() {
            self.selected = self
                .active
                .as_deref()
                .and_then(|path| self.model.index_of(path))
                .or(Some(0));
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Mark `path` as the open file, expanding folders so it can be seen.
    pub fn set_active(&mut self, path: Option<&Path>, cx: &mut Context<Self>) {
        self.active = path.map(|p| p.canonicalize().unwrap_or_else(|_| p.to_path_buf()));
        let lead = self.selected_path();
        if let Some(ix) = path.and_then(|p| self.model.reveal(p)) {
            // Revealing may have opened folders above the selection.
            self.selected = lead.and_then(|lead| self.model.index_of(&lead));
            if !self.is_selected(ix) {
                self.selected = Some(ix);
                self.marked.clear();
                self.anchor = self.selected_path();
            }
            self.scroll_handle
                .scroll_to_item(ix, ScrollStrategy::Center);
        }
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let selected = self.selected_path();
        self.model.refresh();
        self.selected = selected.and_then(|path| self.model.index_of(&path));
        self.forget_missing();
        cx.notify();
    }

    /// Drop selected rows that are gone, from disk or from view.
    fn forget_missing(&mut self) {
        let model = &self.model;
        self.marked.retain(|path| model.index_of(path).is_some());
        if self.marked.len() < 2 {
            self.marked.clear();
        }
    }
}
