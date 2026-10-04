//! Git in the workspace: change bars in each tab's gutter against the last
//! commit, a popup with a change's old lines when one is clicked, the Git
//! panel (⌃⇧G) and the branch picker, and the branch in the title bar.
//!
//! The bars are worked out again on every edit, from the committed text read
//! once when the tab opens and again whenever git may have moved on: the
//! window comes back to the front, a file is saved, or the panel committed,
//! pulled or switched branches.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui_kit::base::input::{LineChange, LineChangeKind};
use gpui_kit::component::input::EditorState;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_editor::EditorHandle as _;

use super::{SwitchBranch, TITLE_BAR_HEIGHT, ToggleGitPanel, Workspace};
use crate::branch_picker::{BranchPicker, BranchPickerEvent};
use crate::document::Document;
use crate::git::{self, Hunk, Repo};
use crate::git_panel::{BRANCH_ICON, GitPanel, GitPanelEvent};

/// Lines of a change the popup shows on each side before cutting it short.
const POPUP_LINES: usize = 16;
const POPUP_WIDTH: f32 = 520.;

#[derive(Default)]
pub(super) struct GitState {
    /// The project's repository, once found.
    repo: Option<Repo>,
    /// The branch checked out, or the commit when none is, for the title bar.
    head: Option<String>,
    /// Each tab's file as last committed, and how the buffer differs.
    files: HashMap<EntityId, TabGit>,
    popup: Option<HunkPopup>,
    panel: Option<Opened<GitPanel>>,
    branches: Option<Opened<BranchPicker>>,
    _refresh: Option<Task<()>>,
    _loads: HashMap<EntityId, Task<()>>,
    _switch: Option<Task<()>>,
}

struct TabGit {
    committed: String,
    hunks: Vec<Hunk>,
}

/// The old lines of one change, shown under it.
struct HunkPopup {
    tab: EntityId,
    hunk: Hunk,
    anchor: Point<Pixels>,
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
        if self.git.popup.as_ref().is_some_and(|popup| popup.tab == id) {
            self.git.popup = None;
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
    pub(super) fn refresh_git(&mut self, cx: &mut Context<Self>) {
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
        self.git._refresh = Some(cx.spawn(async move |this, cx| {
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
            this.update(cx, |this, cx| {
                this.git.repo = repo;
                this.git.head = head;
                for (id, committed) in committed {
                    this.set_committed(id, committed, cx);
                }
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
        if self
            .git
            .popup
            .as_ref()
            .is_some_and(|popup| popup.tab == id && popup.hunk == hunk)
        {
            self.git.popup = None;
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
        self.git.popup = Some(HunkPopup {
            tab: id,
            hunk,
            anchor: point(anchor.x, anchor.y + px(4.)),
        });
        cx.notify();
    }

    /// Esc: close the change popup, if it's open.
    pub(super) fn close_hunk_popup(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.git.popup.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    /// Put the change's lines back as they were committed, as one undo step.
    fn revert_hunk(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(popup) = self.git.popup.take() else {
            return;
        };
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

    pub(super) fn render_hunk_popup(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let popup = self.git.popup.as_ref()?;
        let tab = self.tabs.iter().find(|tab| tab.id() == popup.tab)?;
        let theme = cx.theme();
        let text = tab.editor.text(cx);
        let new_text = &text
            [line_start(&text, popup.hunk.lines.start)..line_start(&text, popup.hunk.lines.end)];
        let title = if popup.hunk.is_deletion() {
            plural(
                popup.hunk.old_text.lines().count(),
                "line removed",
                "lines removed",
            )
        } else if popup.hunk.is_addition() {
            plural(popup.hunk.lines.len(), "line added", "lines added")
        } else {
            plural(popup.hunk.lines.len(), "line changed", "lines changed")
        };
        let lines = |text: &str, sign: &'static str, color: Hsla| {
            let all: Vec<&str> = text.lines().collect();
            let more = all.len().saturating_sub(POPUP_LINES);
            v_flex()
                .children(all.into_iter().take(POPUP_LINES).map(move |line| {
                    h_flex()
                        .px_2()
                        .bg(color.opacity(0.12))
                        .child(div().flex_none().w(px(14.)).text_color(color).child(sign))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .whitespace_nowrap()
                                .child(line.replace('\t', "    ")),
                        )
                }))
                .when(more > 0, |this| {
                    this.child(
                        div()
                            .px_2()
                            .text_color(theme.muted_foreground)
                            .child(format!("… {more} more")),
                    )
                })
        };
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .px_2()
                .h(px(22.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .text_size(px(11.5))
                .text_color(theme.foreground.opacity(0.85))
                .hover(|s| s.bg(theme.foreground.opacity(0.08)))
                .child(label)
        };
        let panel =
            jig_commands::surface::panel(cx)
                .id("git-hunk")
                .w(px(POPUP_WIDTH))
                .max_w(px(POPUP_WIDTH))
                .p_1()
                .flex()
                .flex_col()
                .gap_1()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.close_hunk_popup(cx);
                }))
                .child(
                    h_flex()
                        .pl_2()
                        .gap_1()
                        .child(
                            div()
                                .flex_1()
                                .text_size(px(11.5))
                                .text_color(theme.muted_foreground)
                                .child(format!("{title} since the last commit")),
                        )
                        .child(button("git-revert", "Revert").on_click(
                            cx.listener(|this, _, window, cx| this.revert_hunk(window, cx)),
                        ))
                        .child(button("git-hunk-close", "Close").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.close_hunk_popup(cx);
                            },
                        ))),
                )
                .child(
                    v_flex()
                        .rounded(px(6.))
                        .overflow_hidden()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(12.))
                        .child(lines(&popup.hunk.old_text, "−", theme.red))
                        .child(lines(new_text, "+", theme.green)),
                );
        Some(
            deferred(
                anchored()
                    .position(popup.anchor)
                    .snap_to_window_with_margin(px(8.))
                    .child(panel),
            )
            .into_any_element(),
        )
    }

    /// ⌃⇧G: the Git panel, or closing it.
    pub(super) fn toggle_git_panel(
        &mut self,
        _: &ToggleGitPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.git.panel.is_some() {
            self.close_git_panel(window, cx, true);
        } else {
            self.open_git_panel(window, cx);
        }
    }

    fn open_git_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repo.clone() else {
            self.show_error(
                "This project isn't in a Git repository. Run git init in its folder to start one.",
                window,
                cx,
            );
            return;
        };
        self.palette = None;
        self.quick_open = None;
        self.find_in_files = None;
        self.git.branches = None;
        self.git.popup = None;
        let view = cx.new(|cx| GitPanel::new(repo, window, cx));
        let events = cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
            GitPanelEvent::Open(path) => {
                let path = path.clone();
                this.close_git_panel(window, cx, false);
                this.open_file(&path, window, cx);
            }
            GitPanelEvent::Branches => {
                this.close_git_panel(window, cx, false);
                this.open_branch_picker(window, cx);
            }
            GitPanelEvent::Changed => this.git_changed(window, cx),
            GitPanelEvent::Dismissed => this.close_git_panel(window, cx, true),
        });
        self.git.panel = Some(Opened {
            view,
            _events: events,
        });
        cx.notify();
    }

    fn close_git_panel(&mut self, window: &mut Window, cx: &mut Context<Self>, refocus: bool) {
        if self.git.panel.take().is_some() {
            if refocus {
                self.focus_main(window, cx);
            }
            cx.notify();
        }
    }

    /// After a save: the panel's lists follow.
    pub(super) fn git_saved(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(panel) = &self.git.panel {
            panel.view.update(cx, |panel, cx| panel.refresh(window, cx));
        }
    }

    pub(super) fn switch_branch(
        &mut self,
        _: &SwitchBranch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_branch_picker(window, cx);
    }

    fn open_branch_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.git.repo.clone() else {
            self.show_error("This project isn't in a Git repository.", window, cx);
            return;
        };
        self.palette = None;
        self.quick_open = None;
        self.find_in_files = None;
        self.git.panel = None;
        let view = cx.new(|cx| BranchPicker::new(window, cx));
        let events = cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
            BranchPickerEvent::Switch(branch) => {
                let branch = branch.clone();
                this.close_branch_picker(window, cx, true);
                if !branch.current {
                    this.checkout(move |repo| repo.switch(&branch), window, cx);
                }
            }
            BranchPickerEvent::Create(name) => {
                let name = name.clone();
                this.close_branch_picker(window, cx, true);
                this.checkout(move |repo| repo.create_branch(&name), window, cx);
            }
            BranchPickerEvent::Dismissed => this.close_branch_picker(window, cx, true),
            BranchPickerEvent::Blurred => this.close_branch_picker(window, cx, false),
        });
        self.git.branches = Some(Opened {
            view: view.clone(),
            _events: events,
        });
        cx.spawn(async move |_, cx| {
            let branches = cx
                .background_executor()
                .spawn(async move { repo.branches().unwrap_or_default() })
                .await;
            view.update(cx, |picker, cx| picker.set_branches(branches, cx));
        })
        .detach();
        cx.notify();
    }

    fn close_branch_picker(&mut self, window: &mut Window, cx: &mut Context<Self>, refocus: bool) {
        if self.git.branches.take().is_some() {
            if refocus {
                self.focus_main(window, cx);
            }
            cx.notify();
        }
    }

    /// Save every changed file, then run `switch` in the background, then
    /// let open tabs follow what it did to the files.
    fn checkout(
        &mut self,
        switch: impl FnOnce(&Repo) -> anyhow::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.git.repo.clone() else {
            return;
        };
        if !self.save_all_for_git(window, cx) {
            return;
        }
        self.git._switch = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { switch(&repo) })
                .await;
            this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.show_error(&format!("{error:#}"), window, cx);
                }
                this.git_changed(window, cx);
            })
            .ok();
        }));
    }

    /// Save each changed file that has somewhere to go, so git sees it.
    /// `false`, after saying so, if one couldn't be.
    fn save_all_for_git(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mut failed = Vec::new();
        for ix in 0..self.tabs.len() {
            let tab = &self.tabs[ix];
            let Some(path) = tab.document.path.clone().filter(|_| tab.dirty) else {
                continue;
            };
            let text = tab.editor.text(cx);
            match self.tabs[ix].document.save(&path, &text) {
                Ok(()) => self.tabs[ix].dirty = false,
                Err(_) => failed.push(self.tabs[ix].document.title()),
            }
        }
        self.lsp_saved(cx);
        self.update_title(window);
        if !failed.is_empty() {
            self.show_error(
                &format!("Couldn't save {}, so the branch stays.", failed.join(", ")),
                window,
                cx,
            );
        }
        failed.is_empty()
    }

    /// Git changed files or the branch: open tabs with no unsaved changes
    /// show the files as they now are, and everything git-related refreshes.
    fn git_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for ix in 0..self.tabs.len() {
            let tab = &self.tabs[ix];
            let Some(path) = tab.document.path.clone().filter(|_| !tab.dirty) else {
                continue;
            };
            let Ok(document) = Document::open(&path) else {
                // Not on this branch; the tab keeps what it had.
                continue;
            };
            if document.saved_text == tab.document.saved_text {
                continue;
            }
            self.remember_breakpoints(ix, cx);
            if ix == self.active {
                self.leave_tab(cx);
            }
            self.tabs[ix] = self.new_tab(document, window, cx);
            if ix == self.active {
                self.activate(ix, window, cx);
            }
        }
        self.refresh_git(cx);
        self.refresh_tree(cx);
        cx.notify();
    }

    pub(super) fn render_git_panel(&self, window: &Window) -> Option<AnyElement> {
        let open = self.git.panel.as_ref()?;
        let width = px(crate::git_panel::WIDTH);
        let left = (window.viewport_size().width - width - px(12.)).max(px(8.));
        Some(
            deferred(
                anchored()
                    .position(point(left, px(TITLE_BAR_HEIGHT + 6.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(open.view.clone()),
            )
            .into_any_element(),
        )
    }

    pub(super) fn render_branch_picker(&self, window: &Window) -> Option<AnyElement> {
        let open = self.git.branches.as_ref()?;
        let width = px(crate::branch_picker::WIDTH);
        let left = ((window.viewport_size().width - width) / 2.).max(px(8.));
        Some(
            deferred(
                anchored()
                    .position(point(left, px(TITLE_BAR_HEIGHT + 24.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(open.view.clone()),
            )
            .into_any_element(),
        )
    }

    /// The branch, in the title bar; clicking it opens the Git panel.
    pub(super) fn render_branch_button(&self, cx: &Context<Self>) -> Option<AnyElement> {
        self.git.repo.as_ref()?;
        let head = self.git.head.clone().unwrap_or_else(|| "Git".into());
        let theme = cx.theme();
        Some(
            h_flex()
                .id("git-button")
                .debug_selector(|| "git-button".into())
                .flex_none()
                .ml_2()
                .h(px(26.))
                .max_w(px(200.))
                .px_2()
                .gap_1p5()
                .rounded(px(6.))
                .text_size(px(12.5))
                .text_color(theme.foreground.opacity(0.85))
                .hover(|s| s.bg(theme.foreground.opacity(0.08)))
                .child(
                    Icon::default()
                        .data(BRANCH_ICON)
                        .size(px(13.))
                        .flex_none()
                        .text_color(theme.muted_foreground),
                )
                .child(div().min_w_0().truncate().whitespace_nowrap().child(head))
                // Otherwise the title bar takes the press as a window drag.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.git.panel.is_none() {
                        this.open_git_panel(window, cx);
                    }
                }))
                .into_any_element(),
        )
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
    pub(super) fn hunk_popup_open(&self) -> bool {
        self.git.popup.is_some()
    }

    #[cfg(test)]
    pub(super) fn git_panel(&self) -> Option<Entity<GitPanel>> {
        self.git.panel.as_ref().map(|panel| panel.view.clone())
    }

    #[cfg(test)]
    pub(super) fn branch_picker(&self) -> Option<Entity<BranchPicker>> {
        self.git.branches.as_ref().map(|picker| picker.view.clone())
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

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

#[cfg(test)]
mod tests {
    use super::line_start;

    #[test]
    fn line_starts() {
        let text = "a\nbc\n\nd";
        assert_eq!(line_start(text, 0), 0);
        assert_eq!(line_start(text, 1), 2);
        assert_eq!(line_start(text, 2), 5);
        assert_eq!(line_start(text, 3), 6);
        assert_eq!(line_start(text, 4), text.len());
    }
}
