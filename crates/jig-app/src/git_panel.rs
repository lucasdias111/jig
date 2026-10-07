//! The Git view of the sidebar (⌃⇧G): fetch, pull and push, a commit
//! message, and the changed files, staged and not. Each file has a check:
//! Stage and Commit take the checked ones. It runs git itself, in the
//! background, and tells the workspace when the files or the branch may have
//! changed so open tabs can follow.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::git::{Change, FileStatus, Repo, Status};

const ROW_HEIGHT: f32 = 26.;

/// Lucide "git-branch".
pub const BRANCH_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="6" x2="6" y1="3" y2="15"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/></svg>"#;
pub const PLUS: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14"/><path d="M12 5v14"/></svg>"#;
const CHECK: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>"#;
/// Lucide "refresh-cw".
const FETCH: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"/><path d="M21 3v5h-5"/><path d="M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"/><path d="M8 16H3v5"/></svg>"#;
/// Lucide "arrow-down-to-line".
const PULL: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 17V3"/><path d="m6 11 6 6 6-6"/><path d="M19 21H5"/></svg>"#;
/// Lucide "arrow-up-from-line".
const PUSH: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m18 9-6-6-6 6"/><path d="M12 3v14"/><path d="M5 21h14"/></svg>"#;
const MINUS: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14"/></svg>"#;

pub enum GitPanelEvent {
    /// Open this file, by its full path.
    Open(PathBuf),
    /// Git changed files on disk, or the branch: tabs and markers follow.
    Changed,
    Dismissed,
    /// The Branch button: the workspace shows the branch switcher.
    Branches,
}

/// What a finished action leads to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum After {
    Nothing,
    /// Files on disk may differ: the workspace hears about it.
    FilesChanged,
    /// As files changed, and the message has been used up.
    Committed,
}

/// A line of the file list.
#[derive(Clone, Copy)]
enum Row {
    Header {
        staged: bool,
        count: usize,
    },
    /// A file, by its index in the status.
    File {
        staged: bool,
        index: usize,
    },
}

pub struct GitPanel {
    repo: Arc<Repo>,
    /// `None` until the first `git status` answers.
    status: Option<Status>,
    /// The file list, worked out once per status: only the rows on screen
    /// are drawn, as there can be thousands.
    rows: Vec<Row>,
    /// Checked files, by path. Each file's check is kept across refreshes;
    /// a file seen for the first time starts checked unless it's untracked.
    checked: HashSet<String>,
    /// Every path the panel has shown, to tell new ones.
    known: HashSet<String>,
    message: Entity<InputState>,
    /// What's running, e.g. "Pushing…".
    busy: Option<&'static str>,
    /// Git's answer when the last action failed.
    error: Option<String>,
    scroll: UniformListScrollHandle,
    _task: Option<Task<()>>,
    _subscription: Subscription,
}

impl EventEmitter<GitPanelEvent> for GitPanel {}

impl Focusable for GitPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.message.focus_handle(cx)
    }
}

impl GitPanel {
    pub fn new(repo: Repo, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let message = cx.new(|cx| InputState::new(window, cx).placeholder("Commit message"));
        let subscription = cx.subscribe_in(&message, window, |this, _, event, window, cx| {
            if let InputEvent::PressEnter { .. } = event {
                this.commit(window, cx);
            }
        });
        let mut this = Self {
            repo: Arc::new(repo),
            status: None,
            rows: Vec::new(),
            checked: HashSet::new(),
            known: HashSet::new(),
            message,
            busy: None,
            error: None,
            scroll: UniformListScrollHandle::default(),
            _task: None,
            _subscription: subscription,
        };
        this.refresh(window, cx);
        this
    }

    pub fn root(&self) -> &std::path::Path {
        self.repo.root()
    }

    /// Put the cursor in the commit message.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.message.update(cx, |input, cx| input.focus(window, cx));
    }

    #[cfg(test)]
    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    #[cfg(test)]
    pub fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    #[cfg(test)]
    pub fn set_message(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.message.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
    }

    /// Ask git for the status again, e.g. after a save.
    pub fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_none() {
            self.run(None, After::Nothing, |_| Ok(()), window, cx);
        }
    }

    /// Run `action` in the background, then read the status, then do what
    /// `after` says if it worked.
    fn run(
        &mut self,
        label: Option<&'static str>,
        after: After,
        action: impl FnOnce(&Repo) -> anyhow::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if label.is_some() {
            // One git at a time: two would fight over the index's lock.
            if self.busy.is_some() {
                return;
            }
            self.busy = label;
            self.error = None;
        }
        let repo = self.repo.clone();
        self._task = Some(cx.spawn_in(window, async move |this, cx| {
            let (result, status) = cx
                .background_executor()
                .spawn(async move { (action(&repo), repo.status()) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.busy = None;
                match status {
                    Ok(status) => this.set_status(status),
                    Err(error) => this.error = Some(format!("{error:#}")),
                }
                match result {
                    Err(error) => this.error = Some(format!("{error:#}")),
                    Ok(()) if after == After::Committed => {
                        this.message
                            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
                        cx.emit(GitPanelEvent::Changed);
                    }
                    Ok(()) if after == After::FilesChanged => cx.emit(GitPanelEvent::Changed),
                    Ok(()) => {}
                }
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    fn set_status(&mut self, status: Status) {
        let section = |staged: bool| -> Vec<Row> {
            let files: Vec<Row> = status
                .files
                .iter()
                .enumerate()
                .filter(|(_, file)| {
                    if staged {
                        file.staged.is_some()
                    } else {
                        file.unstaged.is_some()
                    }
                })
                .map(|(index, _)| Row::File { staged, index })
                .collect();
            if files.is_empty() {
                return files;
            }
            let header = Row::Header {
                staged,
                count: files.len(),
            };
            std::iter::once(header).chain(files).collect()
        };
        self.rows = section(true);
        self.rows.extend(section(false));
        let present: HashSet<&str> = status.files.iter().map(|f| f.path.as_str()).collect();
        // A file that leaves the list starts afresh if it comes back.
        self.known.retain(|path| present.contains(path.as_str()));
        self.checked.retain(|path| present.contains(path.as_str()));
        for file in &status.files {
            if self.known.insert(file.path.clone()) && !file.is_untracked() {
                self.checked.insert(file.path.clone());
            }
        }
        self.status = Some(status);
    }

    fn files(&self) -> &[FileStatus] {
        self.status.as_ref().map_or(&[], |s| s.files.as_slice())
    }

    fn is_checked(&self, file: &FileStatus) -> bool {
        self.checked.contains(&file.path)
    }

    /// The checked files, with the old paths of renamed ones so the commit
    /// takes both sides of the rename.
    fn checked_paths(&self) -> Vec<String> {
        self.files()
            .iter()
            .filter(|file| self.is_checked(file))
            .flat_map(|file| std::iter::once(file.path.clone()).chain(file.original.clone()))
            .collect()
    }

    /// The checked files in the staged section, or the other one; for a
    /// rename, its old path too.
    fn checked_in(&self, staged: bool) -> Vec<String> {
        self.section_files(staged)
            .filter(|file| self.is_checked(file))
            .flat_map(|file| {
                let original = file.original.clone().filter(|_| staged);
                std::iter::once(file.path.clone()).chain(original)
            })
            .collect()
    }

    fn toggle(&mut self, path: &str, cx: &mut Context<Self>) {
        if !self.checked.remove(path) {
            self.checked.insert(path.to_string());
        }
        cx.notify();
    }

    /// Check every file in the section, or uncheck them all if they are.
    fn toggle_section(&mut self, staged: bool, cx: &mut Context<Self>) {
        let paths: Vec<String> = self
            .section_files(staged)
            .map(|file| file.path.clone())
            .collect();
        let all = paths.iter().all(|path| self.checked.contains(path));
        for path in paths {
            if all {
                self.checked.remove(&path);
            } else {
                self.checked.insert(path);
            }
        }
        cx.notify();
    }

    fn section_files(&self, staged: bool) -> impl Iterator<Item = &FileStatus> {
        self.files().iter().filter(move |file| {
            if staged {
                file.staged.is_some()
            } else {
                file.unstaged.is_some()
            }
        })
    }

    /// Whether the section's files are all checked, some, or none.
    fn section_check(&self, staged: bool) -> Check {
        let (mut checked, mut total) = (0, 0);
        for file in self.section_files(staged) {
            total += 1;
            checked += usize::from(self.is_checked(file));
        }
        match checked {
            0 => Check::Off,
            n if n == total => Check::On,
            _ => Check::Some,
        }
    }

    #[cfg(test)]
    pub fn checked_files(&self) -> Vec<String> {
        let mut paths = self.checked_paths();
        paths.sort();
        paths
    }

    #[cfg(test)]
    pub fn toggle_file(&mut self, path: &str, cx: &mut Context<Self>) {
        self.toggle(path, cx);
    }

    /// Stage the checked changes, or unstage the checked staged files.
    /// Only checked ones: untracked files start unchecked, so a build
    /// folder never comes along unasked.
    fn stage_checked(&mut self, staged: bool, window: &mut Window, cx: &mut Context<Self>) {
        let paths = self.checked_in(staged);
        if paths.is_empty() {
            return;
        }
        if staged {
            self.unstage(paths, window, cx);
        } else {
            self.stage(paths, window, cx);
        }
    }

    fn stage(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let stage = move |repo: &Repo| repo.stage(&paths);
        self.run(Some("Staging…"), After::Nothing, stage, window, cx);
    }

    fn unstage(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let unstage = move |repo: &Repo| repo.unstage(&paths);
        self.run(Some("Unstaging…"), After::Nothing, unstage, window, cx);
    }

    /// Commit the checked files, as they are now.
    pub fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let message = self.message.read(cx).value().trim().to_string();
        let paths = self.checked_paths();
        let problem = if self.files().is_empty() {
            Some("Nothing to commit.")
        } else if paths.is_empty() {
            Some("Check the files to commit.")
        } else if message.is_empty() {
            Some("Write a commit message first.")
        } else {
            None
        };
        if let Some(problem) = problem {
            self.error = Some(problem.into());
            cx.notify();
            return;
        }
        let commit = move |repo: &Repo| repo.commit(&message, &paths);
        self.run(Some("Committing…"), After::Committed, commit, window, cx);
    }

    fn fetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(Some("Fetching…"), After::Nothing, Repo::fetch, window, cx);
    }

    fn pull(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(
            Some("Pulling…"),
            After::FilesChanged,
            Repo::pull,
            window,
            cx,
        );
    }

    fn push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run(Some("Pushing…"), After::Nothing, Repo::push, window, cx);
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        cx.emit(GitPanelEvent::Dismissed);
    }

    /// The view's title and how far the branch is from its upstream, with
    /// the remote actions on a strip below.
    fn render_header(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (ahead, behind) = self.status.as_ref().map_or((0, 0), |s| (s.ahead, s.behind));
        let counts = [(ahead, "↑"), (behind, "↓")]
            .into_iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, arrow)| format!("{arrow}{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        let action = |id: &'static str, icon: &'static [u8], label: &'static str| {
            h_flex()
                .id(id)
                .debug_selector(move || id.into())
                .flex_1()
                .justify_center()
                .h(px(22.))
                .gap_1()
                .rounded(px(5.))
                .text_size(px(11.5))
                .text_color(theme.muted_foreground)
                .hover(|s| s.bg(theme.sidebar_accent).text_color(theme.foreground))
                .child(Icon::default().data(icon).size(px(12.)).flex_none())
                .child(label)
        };
        let title = h_flex()
            .h(px(22.))
            .px_2()
            .gap_1p5()
            .text_size(px(11.))
            .font_semibold()
            .text_color(theme.muted_foreground)
            .child(div().truncate().child("Changes"))
            .when(!counts.is_empty(), |this| {
                this.child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::NORMAL)
                        .child(counts),
                )
            });
        let actions = h_flex()
            .p_0p5()
            .gap_0p5()
            .rounded(px(7.))
            .border_1()
            .border_color(theme.foreground.opacity(0.08))
            .bg(theme.foreground.opacity(0.03))
            .child(
                action("git-fetch", FETCH, "Fetch")
                    .on_click(cx.listener(|this, _, window, cx| this.fetch(window, cx))),
            )
            .child(
                action("git-pull", PULL, "Pull")
                    .on_click(cx.listener(|this, _, window, cx| this.pull(window, cx))),
            )
            .child(
                action("git-push", PUSH, "Push")
                    .on_click(cx.listener(|this, _, window, cx| this.push(window, cx))),
            )
            .child(
                action("git-branches", BRANCH_ICON, "Branch")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(GitPanelEvent::Branches))),
            );
        v_flex()
            .pt_0p5()
            .gap_1()
            .child(title)
            .child(actions)
            .into_any_element()
    }

    fn render_commit(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let count = self.files().iter().filter(|f| self.is_checked(f)).count();
        let label = match count {
            0 => "Commit".to_string(),
            n => format!("Commit {n}"),
        };
        v_flex()
            .gap_1p5()
            .child(
                div()
                    .w_full()
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(theme.foreground.opacity(0.12))
                    .bg(theme.foreground.opacity(0.03))
                    .text_size(px(13.))
                    .child(
                        Input::new(&self.message)
                            .w_full()
                            .appearance(false)
                            .cleanable(false),
                    ),
            )
            .child(
                div()
                    .id("git-commit")
                    .debug_selector(|| "git-commit".into())
                    .w_full()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .bg(theme.primary)
                    .text_color(theme.primary_foreground)
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .hover(|s| s.opacity(0.9))
                    .child(label)
                    .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx))),
            )
            .into_any_element()
    }

    fn render_row(&self, ix: usize, cx: &Context<Self>) -> AnyElement {
        match self.rows[ix] {
            Row::Header { staged, count } => self.render_header_row(staged, count, cx),
            Row::File { staged, index } => self.render_file(staged, ix, index, cx),
        }
    }

    fn render_header_row(&self, staged: bool, count: usize, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let title = if staged { "Staged" } else { "Changes" };
        let checked = self.checked_in(staged).len();
        let action = match (checked, staged) {
            (0, _) => None,
            (n, true) => Some(format!("Unstage {n}")),
            (n, false) => Some(format!("Stage {n}")),
        };
        h_flex()
            .h(px(ROW_HEIGHT))
            .px_2()
            .gap_2()
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.muted_foreground)
            .child(
                checkbox(self.section_check(staged), cx)
                    .id(if staged {
                        "git-check-staged"
                    } else {
                        "git-check-unstaged"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_section(staged, cx))),
            )
            .child(format!("{title}  {count}"))
            .child(div().flex_1())
            .children(action.map(|label| {
                div()
                    .id(if staged {
                        "git-unstage-checked"
                    } else {
                        "git-stage-checked"
                    })
                    .px_1p5()
                    .rounded(px(4.))
                    .hover(|s| s.bg(theme.foreground.opacity(0.08)))
                    .child(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.stage_checked(staged, window, cx)
                    }))
            }))
            .into_any_element()
    }

    fn render_file(&self, staged: bool, ix: usize, index: usize, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(file) = self.status.as_ref().and_then(|s| s.files.get(index)) else {
            return div().h(px(ROW_HEIGHT)).into_any_element();
        };
        let change = if staged { file.staged } else { file.unstaged };
        let change = change.unwrap_or(Change::Modified);
        let color = match change {
            Change::Added | Change::Untracked => theme.green,
            Change::Deleted | Change::Conflicted => theme.red,
            Change::Modified | Change::Renamed => theme.blue,
        };
        // An untracked folder comes as one entry, ending in `/`.
        let folder = file.path.ends_with('/');
        let path = file.path.trim_end_matches('/');
        let (dir, name) = match path.rsplit_once('/') {
            Some((dir, name)) => (dir.to_string(), name.to_string()),
            None => (String::new(), path.to_string()),
        };
        let name = if folder { format!("{name}/") } else { name };
        let group = SharedString::from(format!("git-file-{ix}"));
        let path = file.path.clone();
        let check = if self.is_checked(file) {
            Check::On
        } else {
            Check::Off
        };
        let full = self.repo.root().join(&file.path);
        h_flex()
            .id(("git-file", ix))
            .group(group.clone())
            .h(px(ROW_HEIGHT))
            .px_2()
            .gap_2()
            .rounded(px(6.))
            .hover(|s| s.bg(theme.foreground.opacity(0.06)))
            .child(checkbox(check, cx).id(("git-check", ix)).on_click({
                let path = path.clone();
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle(&path, cx);
                })
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(12.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .font_family(theme.mono_font_family.clone())
                    .text_color(color)
                    .child(change.letter()),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1p5()
                    .items_baseline()
                    .child(
                        div()
                            .flex_shrink(1.)
                            .min_w(px(40.))
                            .truncate()
                            .text_size(px(13.))
                            .when(change == Change::Deleted, |this| this.line_through())
                            .child(name),
                    )
                    .when(!dir.is_empty(), |this| {
                        this.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(theme.muted_foreground)
                                .child(dir),
                        )
                    }),
            )
            .child(
                div()
                    .id(("git-stage", ix))
                    .flex_none()
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .invisible()
                    .group_hover(group, |s| s.visible())
                    .hover(|s| s.bg(theme.foreground.opacity(0.1)))
                    .child(
                        Icon::default()
                            .data(if staged { MINUS } else { PLUS })
                            .size(px(12.))
                            .text_color(theme.foreground.opacity(0.8)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if staged {
                            this.unstage(vec![path.clone()], window, cx)
                        } else {
                            this.stage(vec![path.clone()], window, cx)
                        }
                    })),
            )
            .when(change != Change::Deleted && !folder, |row| {
                row.on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(GitPanelEvent::Open(full.clone()))),
                )
            })
            .into_any_element()
    }
}

impl Render for GitPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let clean = self.status.is_some() && self.rows.is_empty();
        let rows = self.rows.len();
        let list = uniform_list(
            "git-files",
            rows,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full();
        let header = self.render_header(cx);
        let commit = self.render_commit(cx);
        let theme = cx.theme();
        let footer = match (&self.error, self.busy) {
            (_, Some(busy)) => div()
                .text_size(px(11.5))
                .text_color(theme.muted_foreground)
                .child(busy),
            (Some(error), None) => div()
                .text_size(px(11.5))
                .text_color(theme.danger)
                .child(error.clone()),
            (None, None) => jig_commands::surface::hint("↩ commits the checked files", cx),
        };
        v_flex()
            .id("git-panel")
            .key_context("JigGitPanel")
            .capture_action(cx.listener(Self::on_escape))
            .size_full()
            .px_2()
            .gap_1p5()
            .child(header)
            .child(commit)
            // The list fills the space, so the empty message takes its place
            // rather than following it.
            .child(div().flex_1().min_h_0().map(|this| {
                if clean {
                    this.child(
                        div()
                            .px_2()
                            .py_3()
                            .text_size(px(13.))
                            .text_color(theme.muted_foreground)
                            .child("No changes since the last commit."),
                    )
                } else {
                    this.child(list)
                }
            }))
            .child(div().px_1().pb_2().child(footer))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Check {
    On,
    Off,
    /// Some of a section's files.
    Some,
}

/// A small macOS-style checkbox.
fn checkbox(check: Check, cx: &App) -> Div {
    let theme = cx.theme();
    let filled = check != Check::Off;
    div()
        .flex_none()
        .size(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(if filled {
            theme.primary
        } else {
            theme.foreground.opacity(0.25)
        })
        .when(filled, |this| this.bg(theme.primary))
        .when(!filled, |this| this.bg(theme.foreground.opacity(0.03)))
        .when(filled, |this| {
            this.child(
                Icon::default()
                    .data(if check == Check::On { CHECK } else { MINUS })
                    .size(px(10.))
                    .text_color(theme.primary_foreground),
            )
        })
}
