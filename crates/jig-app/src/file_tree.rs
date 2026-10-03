//! The project sidebar: every folder and file under the project root, read
//! lazily as folders are expanded.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, StyledExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

const CONTEXT: &str = "FileTree";
/// The tree's context while a name is being typed, so its single-key
/// bindings (space, arrows) reach the name field instead.
const NAMING_CONTEXT: &str = "FileTreeNaming";
const ROW_HEIGHT: f32 = 26.;
const INDENT: f32 = 14.;

const CHEVRON_RIGHT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>"#;
const FILE_PLUS: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M15 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V7Z"/><path d="M14 2v4a2 2 0 0 0 2 2h4"/><path d="M9 15h6"/><path d="M12 18v-6"/></svg>"#;
const FOLDER_PLUS: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 10v6"/><path d="M9 13h6"/><path d="M20 20a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.9a2 2 0 0 1-1.69-.9L9.6 3.9A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13a2 2 0 0 0 2 2Z"/></svg>"#;
const FOLDER: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="black"><path d="M3 6.5A2.5 2.5 0 0 1 5.5 4h3.88a2 2 0 0 1 1.42.59L12.2 6H18.5A2.5 2.5 0 0 1 21 8.5v9a2.5 2.5 0 0 1-2.5 2.5h-13A2.5 2.5 0 0 1 3 17.5z"/></svg>"#;
const FILE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linejoin="round"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/></svg>"#;
const CHEVRON_DOWN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;

actions!(
    file_tree,
    [
        SelectPrevious,
        SelectNext,
        CollapseOrParent,
        ExpandOrChild,
        OpenSelected,
        Dismiss,
        CancelNaming
    ]
);

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("up", SelectPrevious, Some(CONTEXT)),
        KeyBinding::new("down", SelectNext, Some(CONTEXT)),
        KeyBinding::new("left", CollapseOrParent, Some(CONTEXT)),
        KeyBinding::new("right", ExpandOrChild, Some(CONTEXT)),
        KeyBinding::new("enter", OpenSelected, Some(CONTEXT)),
        KeyBinding::new("space", OpenSelected, Some(CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(CONTEXT)),
        KeyBinding::new("escape", CancelNaming, Some(NAMING_CONTEXT)),
    ]
}

/// One file or folder in a listing.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    path: PathBuf,
    name: String,
    is_dir: bool,
    /// Matched by a `.gitignore`: shown, but dimmed.
    ignored: bool,
}

/// A visible line of the tree.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub path: PathBuf,
    pub name: String,
    pub depth: usize,
    pub is_dir: bool,
    pub expanded: bool,
    pub ignored: bool,
}

/// The tree without any UI: which folders are open and what they contain.
/// Folders are read the first time they're expanded and again on refresh.
pub struct TreeModel {
    root: PathBuf,
    expanded: HashSet<PathBuf>,
    listings: HashMap<PathBuf, Vec<Entry>>,
    rows: Vec<Row>,
}

impl TreeModel {
    pub fn new(root: &Path) -> Self {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let mut this = Self {
            root,
            expanded: HashSet::new(),
            listings: HashMap::new(),
            rows: Vec::new(),
        };
        this.rebuild();
        this
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn index_of(&self, path: &Path) -> Option<usize> {
        self.rows.iter().position(|row| row.path == path)
    }

    pub fn toggle(&mut self, dir: &Path) {
        if !self.expanded.remove(dir) {
            self.expanded.insert(dir.to_path_buf());
        }
        self.rebuild();
    }

    pub fn set_expanded(&mut self, dir: &Path, expanded: bool) {
        let changed = if expanded {
            self.expanded.insert(dir.to_path_buf())
        } else {
            self.expanded.remove(dir)
        };
        if changed {
            self.rebuild();
        }
    }

    /// Expand every folder between the root and `path`, so it has a row.
    /// Returns that row, or `None` when `path` is outside the project.
    pub fn reveal(&mut self, path: &Path) -> Option<usize> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if !path.starts_with(&self.root) || path == self.root {
            return None;
        }
        let mut changed = false;
        for dir in path.ancestors().skip(1) {
            if dir == self.root {
                break;
            }
            changed |= self.expanded.insert(dir.to_path_buf());
        }
        if changed {
            self.rebuild();
        }
        self.index_of(&path)
    }

    /// Read every open folder again, to pick up changes made on disk.
    pub fn refresh(&mut self) {
        self.listings.clear();
        self.expanded.retain(|dir| dir.is_dir());
        self.rebuild();
    }

    fn rebuild(&mut self) {
        let mut rows = Vec::new();
        let root = self.root.clone();
        self.push_children(&root, 0, false, &mut rows);
        self.rows = rows;
    }

    fn push_children(&mut self, dir: &Path, depth: usize, ignored: bool, rows: &mut Vec<Row>) {
        let entries = self
            .listings
            .entry(dir.to_path_buf())
            .or_insert_with(|| read_listing(&self.root, dir, ignored))
            .clone();
        for entry in entries {
            let expanded = entry.is_dir && self.expanded.contains(&entry.path);
            rows.push(Row {
                path: entry.path.clone(),
                name: entry.name,
                depth,
                is_dir: entry.is_dir,
                expanded,
                ignored: entry.ignored,
            });
            if expanded {
                self.push_children(&entry.path, depth + 1, entry.ignored, rows);
            }
        }
    }
}

/// What the New File and New Folder buttons make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NewEntry {
    File,
    Folder,
}

/// Create `name` inside `dir` and return its path. `name` may contain `/`
/// to create folders on the way, like `src/ui/tree.rs`.
pub fn create_entry(dir: &Path, name: &str, kind: NewEntry) -> Result<PathBuf, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Enter a name.".into());
    }
    let relative = Path::new(name);
    if !relative
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        return Err("Use a name inside this folder.".into());
    }
    let path = dir.join(relative);
    if path.exists() {
        return Err(format!("{name} already exists."));
    }
    let created = match kind {
        NewEntry::Folder => std::fs::create_dir_all(&path),
        NewEntry::File => path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| {
                std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&path)
                    .map(drop)
            }),
    };
    created.map_err(|error| format!("Could not create {name}: {error}"))?;
    Ok(path)
}

/// The folders and files directly inside `dir`, folders first, each group
/// sorted by name ignoring case. Unreadable folders list as empty.
fn read_listing(root: &Path, dir: &Path, parent_ignored: bool) -> Vec<Entry> {
    let Ok(read) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let gitignore = if parent_ignored {
        Gitignore::empty()
    } else {
        gitignore_for(root, dir)
    };
    let mut entries: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter_map(|item| {
            let name = item.file_name().to_string_lossy().into_owned();
            if name == ".git" || name == ".DS_Store" {
                return None;
            }
            let path = item.path();
            // Follows symlinks, so a link to a folder expands like one.
            let is_dir = path.is_dir();
            let ignored = parent_ignored || gitignore.matched(&path, is_dir).is_ignore();
            Some(Entry {
                path,
                name,
                is_dir,
                ignored,
            })
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dir
            .cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    entries
}

/// The `.gitignore` rules that apply inside `dir`, from the root down.
fn gitignore_for(root: &Path, dir: &Path) -> Gitignore {
    let mut builder = GitignoreBuilder::new(root);
    let mut dirs: Vec<&Path> = dir
        .ancestors()
        .take_while(|d| d.starts_with(root))
        .collect();
    dirs.reverse();
    for dir in dirs {
        let file = dir.join(".gitignore");
        if file.is_file() {
            builder.add(file);
        }
    }
    builder.build().unwrap_or_else(|_| Gitignore::empty())
}

pub enum FileTreeEvent {
    /// A file was chosen.
    Open(PathBuf),
    /// Escape: hand focus back to the editor.
    Dismissed,
}

impl EventEmitter<FileTreeEvent> for FileTree {}

/// The name field shown in the tree while making a file or folder.
struct Naming {
    /// The folder the new entry goes in.
    dir: PathBuf,
    kind: NewEntry,
    input: Entity<InputState>,
    error: Option<String>,
    _events: Subscription,
}

/// The sidebar view.
pub struct FileTree {
    model: TreeModel,
    naming: Option<Naming>,
    selected: Option<usize>,
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
            active: None,
            focus_handle: cx.focus_handle(),
            scroll_handle: UniformListScrollHandle::default(),
        }
    }

    pub fn root(&self) -> &Path {
        self.model.root()
    }

    #[cfg(test)]
    pub fn naming_error(&self) -> Option<Option<String>> {
        self.naming.as_ref().map(|naming| naming.error.clone())
    }

    #[cfg(test)]
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
        if let Some(ix) = path.and_then(|p| self.model.reveal(p)) {
            self.selected = Some(ix);
            self.scroll_handle
                .scroll_to_item(ix, ScrollStrategy::Center);
        }
        cx.notify();
    }

    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let selected = self.selected_path();
        self.model.refresh();
        self.selected = selected.and_then(|path| self.model.index_of(&path));
        cx.notify();
    }

    fn selected_path(&self) -> Option<PathBuf> {
        self.selected
            .and_then(|ix| self.model.rows().get(ix))
            .map(|row| row.path.clone())
    }

    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        self.selected = Some(ix);
        self.scroll_handle
            .scroll_to_item(ix, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Open a file, or expand or collapse a folder.
    fn activate(&mut self, ix: usize, cx: &mut Context<Self>) {
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

    fn select_previous(&mut self, _: &SelectPrevious, _: &mut Window, cx: &mut Context<Self>) {
        let ix = self.selected.map_or(0, |ix| ix.saturating_sub(1));
        self.select(ix, cx);
    }

    fn select_next(&mut self, _: &SelectNext, _: &mut Window, cx: &mut Context<Self>) {
        let last = self.model.rows().len().saturating_sub(1);
        let ix = self.selected.map_or(0, |ix| (ix + 1).min(last));
        self.select(ix, cx);
    }

    fn collapse_or_parent(&mut self, _: &CollapseOrParent, _: &mut Window, cx: &mut Context<Self>) {
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

    fn expand_or_child(&mut self, _: &ExpandOrChild, _: &mut Window, cx: &mut Context<Self>) {
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

    fn open_selected(&mut self, _: &OpenSelected, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.selected {
            self.activate(ix, cx);
        }
    }

    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(FileTreeEvent::Dismissed);
    }

    /// Where a new entry goes: the selected folder, the selected file's
    /// folder, or the root.
    fn target_dir(&self) -> PathBuf {
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
        let events = cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
            InputEvent::PressEnter { .. } => this.finish_new(window, cx),
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
            kind,
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

    fn finish_new(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(naming) = &mut self.naming else {
            return;
        };
        let name = naming.input.read(cx).value().to_string();
        match create_entry(&naming.dir, &name, naming.kind) {
            Ok(path) => {
                let kind = naming.kind;
                self.naming = None;
                self.model.refresh();
                self.selected = self.model.reveal(&path);
                self.focus_handle.focus(window, cx);
                if kind == NewEntry::File {
                    cx.emit(FileTreeEvent::Open(path));
                }
            }
            Err(error) => naming.error = Some(error),
        }
        cx.notify();
    }

    fn cancel_new(&mut self, cx: &mut Context<Self>) {
        if self.naming.take().is_some() {
            cx.notify();
        }
    }

    fn on_naming_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if self.naming.is_none() {
            cx.propagate();
            return;
        }
        self.cancel_naming(&CancelNaming, window, cx);
        cx.stop_propagation();
    }

    fn cancel_naming(&mut self, _: &CancelNaming, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_new(cx);
        self.focus_handle.focus(window, cx);
    }

    /// The list position of the name field: first in its folder.
    fn naming_index(&self) -> Option<usize> {
        let naming = self.naming.as_ref()?;
        if naming.dir == self.model.root() {
            return Some(0);
        }
        self.model.index_of(&naming.dir).map(|ix| ix + 1)
    }

    fn naming_depth(&self) -> usize {
        let Some(naming) = &self.naming else { return 0 };
        self.model
            .index_of(&naming.dir)
            .map_or(0, |ix| self.model.rows()[ix].depth + 1)
    }
}

fn header_button(
    id: &'static str,
    icon: &'static [u8],
    tooltip: &'static str,
    cx: &Context<FileTree>,
    on_click: impl Fn(&mut FileTree, &mut Window, &mut Context<FileTree>) + 'static,
) -> AnyElement {
    let theme = cx.theme();
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(22.))
        .rounded(px(5.))
        .text_color(theme.muted_foreground)
        .hover(|s| s.bg(theme.sidebar_accent).text_color(theme.foreground))
        .child(Icon::default().data(icon).size(px(14.)))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            on_click(this, window, cx)
        }))
        .into_any_element()
}

impl Render for FileTree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        let root_name = self
            .model
            .root()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let naming_at = self.naming_index();
        let rows = self.model.rows().len() + usize::from(naming_at.is_some());
        let new_file = header_button("new-file", FILE_PLUS, "New File", cx, |this, window, cx| {
            this.start_new(NewEntry::File, window, cx)
        });
        let new_folder = header_button(
            "new-folder",
            FOLDER_PLUS,
            "New Folder",
            cx,
            |this, window, cx| this.start_new(NewEntry::Folder, window, cx),
        );
        let theme = cx.theme();
        let error = self.naming.as_ref().and_then(|n| n.error.clone());
        let list =
            uniform_list(
                "file-tree-rows",
                rows,
                cx.processor(move |this, range: Range<usize>, _, cx| {
                    let theme = cx.theme();
                    range
                        .map(|list_ix| {
                            if Some(list_ix) == naming_at
                                && let Some(naming) = &this.naming
                            {
                                let depth = this.naming_depth();
                                return h_flex()
                                    .id("new-entry")
                                    .h(px(ROW_HEIGHT))
                                    .w_full()
                                    .pl(px(4. + depth as f32 * INDENT))
                                    .pr_1()
                                    .gap_1()
                                    .child(div().w(px(12.)).flex_none())
                                    .child(div().w(px(15.)).flex_none())
                                    .child(div().flex_1().min_w_0().child(
                                        Input::new(&naming.input).xsmall().cleanable(false),
                                    ));
                            }
                            let ix = match naming_at {
                                Some(at) if list_ix > at => list_ix - 1,
                                _ => list_ix,
                            };
                            let row = &this.model.rows()[ix];
                            let selected = this.selected == Some(ix);
                            let active = this.active.as_ref() == Some(&row.path);
                            // Like a Finder sidebar: the accent fill marks the
                            // keyboard selection while the tree has focus, a
                            // quiet grey one the open file.
                            let highlighted = selected && focused;
                            let color = if highlighted {
                                theme.sidebar_primary_foreground
                            } else if row.ignored {
                                theme.muted_foreground.opacity(0.6)
                            } else if active {
                                theme.sidebar_accent_foreground
                            } else {
                                theme.sidebar_foreground
                            };
                            let icon_color = if highlighted {
                                theme.sidebar_primary_foreground
                            } else if row.ignored {
                                theme.muted_foreground.opacity(0.5)
                            } else if row.is_dir {
                                theme.primary
                            } else {
                                theme.muted_foreground
                            };
                            let chevron = row.is_dir.then(|| {
                                Icon::default()
                                    .data(if row.expanded {
                                        CHEVRON_DOWN
                                    } else {
                                        CHEVRON_RIGHT
                                    })
                                    .size(px(10.))
                                    .text_color(if highlighted {
                                        theme.sidebar_primary_foreground
                                    } else {
                                        theme.muted_foreground
                                    })
                            });
                            h_flex()
                                .id(list_ix)
                                .h(px(ROW_HEIGHT))
                                .w_full()
                                .pl(px(4. + row.depth as f32 * INDENT))
                                .pr_2()
                                .gap_1()
                                .rounded(px(6.))
                                .text_size(px(13.))
                                .text_color(color)
                                .when(active && !highlighted, |this| this.bg(theme.sidebar_accent))
                                .when(highlighted, |this| this.bg(theme.sidebar_primary))
                                .when(!highlighted && !active, |this| {
                                    this.hover(|s| s.bg(theme.sidebar_accent.opacity(0.5)))
                                })
                                .child(
                                    div()
                                        .w(px(12.))
                                        .flex_none()
                                        .flex()
                                        .justify_center()
                                        .children(chevron),
                                )
                                .child(
                                    Icon::default()
                                        .data(if row.is_dir { FOLDER } else { FILE })
                                        .size(px(15.))
                                        .flex_none()
                                        .text_color(icon_color),
                                )
                                .child(div().pl_0p5().truncate().child(row.name.clone()))
                                // Select on press, as Finder does. Pressing focuses
                                // the tree, which would otherwise light up the
                                // previous selection until the click completes.
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, _, _, cx| this.select(ix, cx)),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.focus_handle.focus(window, cx);
                                    this.activate(ix, cx);
                                }))
                        })
                        .collect()
                }),
            )
            .track_scroll(&self.scroll_handle)
            .size_full()
            .px_2();

        div()
            .id("file-tree")
            .key_context(if self.naming.is_some() {
                NAMING_CONTEXT
            } else {
                CONTEXT
            })
            .track_focus(&self.focus_handle)
            .capture_action(cx.listener(Self::on_naming_escape))
            .on_action(cx.listener(Self::cancel_naming))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::collapse_or_parent))
            .on_action(cx.listener(Self::expand_or_child))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::dismiss))
            .flex()
            .flex_col()
            .size_full()
            .child(
                h_flex()
                    .pl_4()
                    .pr_2()
                    .pt_0p5()
                    .pb_1()
                    .gap_0p5()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(11.))
                            .font_semibold()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(root_name),
                    )
                    .child(new_file)
                    .child(new_folder),
            )
            .when_some(error, |this, error| {
                this.child(
                    div()
                        .mx_2()
                        .mb_1()
                        .px_2()
                        .py_1()
                        .rounded(px(4.))
                        .text_xs()
                        .text_color(theme.danger)
                        .bg(theme.danger.opacity(0.1))
                        .child(error),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .child(list)
                    .vertical_scrollbar(&self.scroll_handle),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{NewEntry, TreeModel, create_entry};

    fn names(model: &TreeModel) -> Vec<String> {
        model
            .rows()
            .iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth), row.name))
            .collect()
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("src/ui")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::create_dir_all(root.join("empty")).unwrap();
        fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
        fs::write(root.join("Cargo.toml"), "").unwrap();
        fs::write(root.join("build.log"), "").unwrap();
        fs::write(root.join("README.md"), "").unwrap();
        fs::write(root.join("src/main.rs"), "").unwrap();
        fs::write(root.join("src/ui/tree.rs"), "").unwrap();
        fs::write(root.join("target/debug/jig"), "").unwrap();
        dir
    }

    #[test]
    fn folders_first_then_files_without_git() {
        let dir = project();
        let model = TreeModel::new(dir.path());
        assert_eq!(
            names(&model),
            [
                "empty",
                "src",
                "target",
                ".gitignore",
                "build.log",
                "Cargo.toml",
                "README.md"
            ]
        );
        let empty = &model.rows()[0];
        assert!(empty.is_dir, "an empty folder is still a folder");
    }

    #[test]
    fn expanding_reads_nested_folders() {
        let dir = project();
        let mut model = TreeModel::new(dir.path());
        let src = model.rows()[1].path.clone();
        model.toggle(&src);
        model.toggle(&src.join("ui"));
        assert_eq!(
            names(&model)[..5],
            ["empty", "src", "  ui", "    tree.rs", "  main.rs"]
        );
        model.toggle(&src);
        assert_eq!(names(&model).len(), 7, "collapsing hides the subtree");
        model.toggle(&src);
        assert!(
            names(&model).contains(&"    tree.rs".to_string()),
            "nested folders stay open"
        );
    }

    #[test]
    fn ignored_entries_are_dimmed_down_the_tree() {
        let dir = project();
        let mut model = TreeModel::new(dir.path());
        let ignored = |model: &TreeModel, name: &str| {
            model
                .rows()
                .iter()
                .find(|row| row.name == name)
                .unwrap()
                .ignored
        };
        assert!(ignored(&model, "target"));
        assert!(ignored(&model, "build.log"));
        assert!(!ignored(&model, "src"));
        let target = model.root().join("target");
        model.toggle(&target);
        assert!(ignored(&model, "debug"), "inside an ignored folder");
    }

    #[test]
    fn reveal_expands_ancestors() {
        let dir = project();
        let mut model = TreeModel::new(dir.path());
        let ix = model.reveal(&dir.path().join("src/ui/tree.rs")).unwrap();
        assert_eq!(model.rows()[ix].name, "tree.rs");
        assert_eq!(model.rows()[ix].depth, 2);
        assert_eq!(model.reveal(Path::new("/elsewhere/x.rs")), None);
    }

    #[test]
    fn creates_files_and_folders() {
        let dir = project();
        let root = dir.path();
        let file = create_entry(root, " notes.md ", NewEntry::File).unwrap();
        assert_eq!(file, root.join("notes.md"));
        assert!(file.is_file());
        let nested = create_entry(root, "src/ui/tabs.rs", NewEntry::File).unwrap();
        assert!(nested.is_file(), "folders on the way are created");
        assert!(
            create_entry(root, "docs/guides", NewEntry::Folder)
                .unwrap()
                .is_dir()
        );

        assert_eq!(
            create_entry(root, "notes.md", NewEntry::File),
            Err("notes.md already exists.".into())
        );
        assert!(create_entry(root, "  ", NewEntry::File).is_err());
        assert!(create_entry(root, "../out.rs", NewEntry::File).is_err());
        assert!(create_entry(root, "/tmp/out.rs", NewEntry::Folder).is_err());
    }

    #[test]
    fn refresh_picks_up_changes_on_disk() {
        let dir = project();
        let mut model = TreeModel::new(dir.path());
        fs::write(dir.path().join("new.rs"), "").unwrap();
        fs::remove_dir(dir.path().join("empty")).unwrap();
        assert!(!names(&model).contains(&"new.rs".to_string()));
        model.refresh();
        assert!(names(&model).contains(&"new.rs".to_string()));
        assert!(!names(&model).contains(&"empty".to_string()));
    }
}
