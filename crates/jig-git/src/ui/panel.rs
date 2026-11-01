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

use crate::domain::status::{Change, FileStatus, Status};
use crate::store::persist::Repo;
use jig_commands::surface::PLUS_ICON as PLUS;

const ROW_HEIGHT: f32 = 26.;

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

    #[cfg(any(test, feature = "testing"))]
    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn is_busy(&self) -> bool {
        self.busy.is_some()
    }

    #[cfg(any(test, feature = "testing"))]
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

    #[cfg(any(test, feature = "testing"))]
    pub fn checked_files(&self) -> Vec<String> {
        let mut paths = self.checked_paths();
        paths.sort();
        paths
    }

    #[cfg(any(test, feature = "testing"))]
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
}

mod icons;
mod view;

pub use icons::BRANCH_ICON;
use icons::*;

use view::Check;
