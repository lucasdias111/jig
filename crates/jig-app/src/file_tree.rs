//! The project sidebar: every folder and file under the project root, read
//! lazily as folders are expanded.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::{ActiveTheme as _, Icon, StyledExt as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

const CONTEXT: &str = "FileTree";
const ROW_HEIGHT: f32 = 24.;
const INDENT: f32 = 12.;

const CHEVRON_RIGHT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>"#;
const CHEVRON_DOWN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;

actions!(
    file_tree,
    [
        SelectPrevious,
        SelectNext,
        CollapseOrParent,
        ExpandOrChild,
        OpenSelected,
        Dismiss
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

/// The sidebar view.
pub struct FileTree {
    model: TreeModel,
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
}

impl Render for FileTree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        let root_name = self
            .model
            .root()
            .file_name()
            .map(|name| name.to_string_lossy().to_uppercase())
            .unwrap_or_default();
        let theme = cx.theme();
        let list = uniform_list(
            "file-tree-rows",
            self.model.rows().len(),
            cx.processor(move |this, range: Range<usize>, _, cx| {
                let theme = cx.theme();
                range
                    .map(|ix| {
                        let row = &this.model.rows()[ix];
                        let selected = this.selected == Some(ix);
                        let active = this.active.as_ref() == Some(&row.path);
                        let color = if row.ignored {
                            theme.muted_foreground.opacity(0.6)
                        } else if active {
                            theme.foreground
                        } else {
                            theme.sidebar_foreground
                        };
                        let chevron = row.is_dir.then(|| {
                            Icon::default()
                                .data(if row.expanded {
                                    CHEVRON_DOWN
                                } else {
                                    CHEVRON_RIGHT
                                })
                                .size(px(12.))
                                .text_color(theme.muted_foreground)
                        });
                        h_flex()
                            .id(ix)
                            .h(px(ROW_HEIGHT))
                            .w_full()
                            .pl(px(8. + row.depth as f32 * INDENT))
                            .pr_2()
                            .gap_1()
                            .rounded(px(4.))
                            .text_sm()
                            .text_color(color)
                            .when(active && !(selected && focused), |this| {
                                this.bg(theme.sidebar_accent)
                            })
                            .when(selected && focused, |this| this.bg(theme.list_active))
                            .when(!selected, |this| this.hover(|s| s.bg(theme.list_hover)))
                            .child(div().w(px(12.)).flex_none().children(chevron))
                            .child(div().truncate().child(row.name.clone()))
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
        .px_1();

        div()
            .id("file-tree")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::collapse_or_parent))
            .on_action(cx.listener(Self::expand_or_child))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::dismiss))
            .flex()
            .flex_col()
            .size_full()
            .bg(theme.sidebar)
            .child(
                div()
                    .px_3()
                    .pb_1p5()
                    .text_xs()
                    .font_semibold()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(root_name),
            )
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

    use super::TreeModel;

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
