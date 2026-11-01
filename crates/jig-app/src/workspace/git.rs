//! Git in the workspace: change bars in each tab's gutter against the last
//! commit, a popup with a change's old lines when one is clicked, the Git
//! view of the sidebar (⌃⇧G), and the branch switcher in the status bar.
//!
//! The bars are worked out again on every edit, from the committed text read
//! once when the tab opens and again whenever git may have moved on: the
//! window comes back to the front, a file is saved, or the panel committed,
//! pulled or switched branches. While the Git view shows, the repository is
//! watched too (`watch.rs`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui_kit::base::input::{LineChange, LineChangeKind};
use gpui_kit::component::input::EditorState;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex};
use gpui_kit::*;
use jig_editor::EditorHandle as _;

mod checkout;
mod view;

pub(super) mod branches;
pub(super) mod hunk;
pub(super) mod watch;

use super::sidebar::SidebarView;
use super::{NewBranch, SwitchBranch, ToggleGitPanel, Workspace};
use branches::{BranchSwitcher, BranchSwitcherEvent};
use hunk::{HunkPopup, RevertHunk};
use jig_editing::document::Document;
use jig_git::BranchPickerMode;
use jig_git::{self as git, Hunk, Repo};
use jig_git::{BRANCH_ICON, GitPanel, GitPanelEvent};

#[derive(Default)]
pub(super) struct GitState {
    /// The project's repository, once found.
    repo: Option<Repo>,
    /// The branch checked out, or the commit when none is, for the status bar.
    head: Option<String>,
    /// Each tab's file as last committed, and how the buffer differs.
    files: HashMap<EntityId, TabGit>,
    /// The sidebar's Git view, kept while the sidebar shows something else
    /// so a half-written commit message survives.
    panel: Option<Opened<GitPanel>>,
    /// The repository watched while the Git view shows.
    watch: Option<watch::RepoWatch>,
    _refresh: Option<Task<()>>,
    _loads: HashMap<EntityId, Task<()>>,
    _switch: Option<Task<()>>,
}

struct TabGit {
    committed: String,
    hunks: Vec<Hunk>,
}

struct Opened<V> {
    view: Entity<V>,
    _events: Subscription,
}

impl Workspace {
    /// Change bars for a new tab's editor, once git says what was committed,
    /// and clicks on them opening the popup.
    pub(super) fn install_git(
        &mut self,
        state: &Entity<EditorState>,
        document: &Document,
        cx: &mut Context<Self>,
    ) {
        let id = state.entity_id();
        let workspace = cx.entity().downgrade();
        state.update(cx, |state, _| {
            state.on_line_change_click(move |line, window, cx| {
                workspace
                    .update(cx, |this, cx| this.show_hunk(id, line, window, cx))
                    .ok();
            });
        });
        if let Some(path) = document.path.clone() {
            self.load_committed(id, path, cx);
        }
    }

    /// Read `path` as last committed, in the background, for tab `id`.
    fn load_committed(&mut self, id: EntityId, path: PathBuf, cx: &mut Context<Self>) {
        let task = cx.spawn(async move |this, cx| {
            let committed = cx
                .background_executor()
                .spawn(async move { Repo::discover(&path)?.committed_text(&path) })
                .await;
            this.update(cx, |this, cx| this.set_committed(id, committed, cx))
                .ok();
        });
        self.git._loads.insert(id, task);
    }

    fn set_committed(&mut self, id: EntityId, committed: Option<String>, cx: &mut Context<Self>) {
        self.git._loads.remove(&id);
        match committed {
            Some(committed) => {
                self.git.files.insert(
                    id,
                    TabGit {
                        committed,
                        hunks: Vec::new(),
                    },
                );
            }
            None => {
                self.git.files.remove(&id);
            }
        }
        if let Some(ix) = self.tabs.iter().position(|tab| tab.id() == id) {
            self.update_line_changes(ix, cx);
        }
    }

    /// After an edit in tab `ix`: its bars follow, and a popup about it
    /// would be out of date.
    pub(super) fn git_buffer_changed(&mut self, ix: usize, cx: &mut Context<Self>) {
        let id = self.tabs[ix].id();
        if self.hunk_popup.read(cx).tab() == Some(id) {
            self.hunk_popup.update(cx, |popup, cx| popup.close(cx));
        }
        self.update_line_changes(ix, cx);
    }

    fn update_line_changes(&mut self, ix: usize, cx: &mut Context<Self>) {
        let tab = &self.tabs[ix];
        let changes = match self.git.files.get_mut(&tab.id()) {
            Some(file) => {
                file.hunks = git::hunks(&file.committed, &tab.editor.text(cx));
                let theme = cx.theme();
                file.hunks
                    .iter()
                    .map(|hunk| line_change(hunk, theme))
                    .collect()
            }
            None => Vec::new(),
        };
        tab.editor
            .state()
            .update(cx, |state, cx| state.set_line_changes(changes, cx));
    }

    /// Find the project's repository and branch again, and what each open
    /// file was at the last commit, e.g. after a commit made elsewhere.
    pub(super) fn refresh_git(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let open: Vec<EntityId> = self.tabs.iter().map(|tab| tab.id()).collect();
        self.git.files.retain(|id, _| open.contains(id));
        let anchor = self
            .project_root(cx)
            .or_else(|| self.document().path.clone());
        let tabs: Vec<(EntityId, PathBuf)> = self
            .tabs
            .iter()
            .filter_map(|tab| Some((tab.id(), tab.document.path.clone()?)))
            .collect();
        self.git._refresh = Some(cx.spawn_in(window, async move |this, cx| {
            let (repo, head, committed) = cx
                .background_executor()
                .spawn(async move {
                    let repo = anchor.as_deref().and_then(Repo::discover);
                    let head = repo.as_ref().and_then(|repo| {
                        let status = repo.status().ok()?;
                        status.branch.or(status.commit)
                    });
                    let mut repos: Vec<Repo> = repo.iter().cloned().collect();
                    let committed: Vec<(EntityId, Option<String>)> = tabs
                        .into_iter()
                        .map(|(id, path)| (id, committed_text(&mut repos, &path)))
                        .collect();
                    (repo, head, committed)
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                let root = repo.as_ref().map(|repo| repo.root().to_path_buf());
                let panel_root = this
                    .git
                    .panel
                    .as_ref()
                    .map(|panel| panel.view.read(cx).root().to_path_buf());
                if panel_root.is_some() && panel_root != root {
                    // Another project: its panel would show the wrong files.
                    this.git.panel = None;
                }
                this.git.repo = repo;
                this.git.head = head;
                for (id, committed) in committed {
                    this.set_committed(id, committed, cx);
                }
                // E.g. the window came forward after a commit in a terminal:
                // the Git view shows where things are now.
                if this.git_view_shown() && !this.ensure_git_panel(window, cx) {
                    this.refresh_git_panel(window, cx);
                }
                this.sync_git_watch(window, cx);
                cx.notify();
            })
            .ok();
        }));
    }

    /// Open the popup for the change starting at `line` in tab `id`.
    fn show_hunk(
        &mut self,
        id: EntityId,
        line: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.tabs.iter().position(|tab| tab.id() == id) else {
            return;
        };
        let Some(hunk) = self.git.files.get(&id).and_then(|file| {
            file.hunks
                .iter()
                .find(|hunk| hunk.lines.start == line)
                .cloned()
        }) else {
            return;
        };
        if self.hunk_popup.read(cx).shows(id, &hunk) {
            self.hunk_popup.update(cx, |popup, cx| popup.close(cx));
            cx.notify();
            return;
        }
        if ix != self.active {
            self.activate(ix, window, cx);
        }
        let state = self.tabs[ix].editor.state().read(cx);
        let text = state.value();
        let below = line_start(&text, hunk.lines.end);
        let at = if below >= text.len() {
            state
                .range_to_bounds(&(text.len()..text.len()))
                .map(|b| point(b.left(), b.bottom()))
        } else {
            state
                .range_to_bounds(&(below..below))
                .map(|b| point(b.left(), b.top()))
        };
        let Some(anchor) = at else {
            return;
        };
        let new_text = text[line_start(&text, hunk.lines.start)..below].to_string();
        let anchor = point(anchor.x, anchor.y + px(4.));
        self.hunk_popup
            .update(cx, |popup, cx| popup.open(id, hunk, new_text, anchor, cx));
        cx.notify();
    }

    /// Esc: close the change popup, if it's open.
    pub(super) fn close_hunk_popup(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.hunk_popup.update(cx, |popup, cx| popup.close(cx));
        if open {
            cx.notify();
        }
        open
    }

    /// Put the change's lines back as they were committed, as one undo step.
    pub(super) fn revert_hunk(
        &mut self,
        _: &Entity<HunkPopup>,
        popup: &RevertHunk,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.tabs.iter().position(|tab| tab.id() == popup.tab) else {
            return;
        };
        if self.previewing() {
            return;
        }
        let editor = self.tabs[ix].editor.clone();
        let text = editor.text(cx);
        let range =
            line_start(&text, popup.hunk.lines.start)..line_start(&text, popup.hunk.lines.end);
        editor.apply_edit(range, &popup.hunk.old_text, window, cx);
        editor.focus(window, cx);
        cx.notify();
    }

    #[cfg(test)]
    pub(super) fn line_change_rows(
        &self,
        cx: &App,
    ) -> Vec<(std::ops::Range<usize>, LineChangeKind)> {
        self.git
            .files
            .get(&self.tab().id())
            .map(|file| {
                file.hunks
                    .iter()
                    .map(|hunk| (hunk.lines.clone(), line_change(hunk, cx.theme()).kind))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn hunk_popup_open(&self, cx: &App) -> bool {
        self.hunk_popup.read(cx).is_open()
    }

    #[cfg(test)]
    pub(super) fn git_panel(&self) -> Option<Entity<GitPanel>> {
        self.git.panel.as_ref().map(|panel| panel.view.clone())
    }

    #[cfg(test)]
    pub(super) fn branch_picker(&self, cx: &App) -> Option<Entity<jig_git::BranchPicker>> {
        self.branch_switcher.read(cx).picker().cloned()
    }
}

/// `path` as committed in whichever of `repos` holds it, finding its
/// repository first if none does.
fn committed_text(repos: &mut Vec<Repo>, path: &Path) -> Option<String> {
    if let Some(repo) = repos.iter().find(|repo| path.starts_with(repo.root())) {
        return repo.committed_text(path);
    }
    let repo = Repo::discover(path)?;
    let text = repo.committed_text(path);
    repos.push(repo);
    text
}

fn line_change(hunk: &Hunk, theme: &gpui_kit::component::Theme) -> LineChange {
    let (kind, color) = if hunk.is_deletion() {
        (LineChangeKind::Deleted, theme.red)
    } else if hunk.is_addition() {
        (LineChangeKind::Added, theme.green)
    } else {
        (LineChangeKind::Modified, theme.blue)
    };
    LineChange {
        rows: hunk.lines.clone(),
        kind,
        color: color.opacity(0.85),
    }
}

/// Where line `line` (0-based) starts; the end of the text past the last.
fn line_start(text: &str, line: usize) -> usize {
    if line == 0 {
        return 0;
    }
    text.match_indices('\n')
        .nth(line - 1)
        .map_or(text.len(), |(ix, _)| ix + 1)
}

#[cfg(test)]
mod tests;
