//! Keeping the Git view current while it shows: the repository is watched,
//! and each burst of changes on disk, from Jig, another editor, a build or
//! git in a terminal, leads to one `git status`. Only while the view is
//! open, as watching a big repository can use up Linux's inotify watches.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::*;
use notify::Watcher as _;

use super::super::Workspace;
use super::super::sidebar::SidebarView;

/// A build writes thousands of files; wait this long after the first change
/// for the rest, then ask git once.
const SETTLE: Duration = Duration::from_millis(300);

pub(in crate::workspace) struct RepoWatch {
    root: PathBuf,
    _watcher: notify::RecommendedWatcher,
    _task: Task<()>,
}

/// What a change on disk means for the Git view, least to most.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(in crate::workspace) enum Touched {
    /// Git's own bookkeeping, which would otherwise trigger itself.
    Nothing,
    /// The working tree: the changed files may differ.
    Files,
    /// HEAD, the index or a ref: a commit, stage or branch switch made
    /// elsewhere, so the branch and the committed text may differ too.
    Git,
}

/// What a change to `path` means. Inside `.git` only HEAD, the index and
/// refs count; the rest is git writing objects, logs and lock files.
fn touched(path: &Path) -> Touched {
    let mut parts = path.components().map(|part| part.as_os_str());
    while let Some(part) = parts.next() {
        if part == ".git" {
            let rest: Vec<_> = parts.collect();
            return match rest.as_slice() {
                [name] if *name == "HEAD" || *name == "index" => Touched::Git,
                [first, ..] if *first == "refs" => Touched::Git,
                _ => Touched::Nothing,
            };
        }
    }
    Touched::Files
}

impl RepoWatch {
    /// Watch `root` and everything under it. `None` if it can't be watched,
    /// e.g. out of inotify watches; the view still refreshes when the window
    /// comes forward. Off in tests, whose own writes would race it.
    fn start(root: PathBuf, window: &mut Window, cx: &mut Context<Workspace>) -> Option<Self> {
        if cfg!(test) {
            return None;
        }
        let (sender, mut changes) = mpsc::unbounded::<Touched>();
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                // Reading a file is an event too on Linux; only changes count.
                let Ok(event) = event else {
                    return;
                };
                if event.kind.is_access() {
                    return;
                }
                let most = event.paths.iter().map(|path| touched(path)).max();
                if let Some(most) = most.filter(|most| *most != Touched::Nothing) {
                    let _ = sender.unbounded_send(most);
                }
            })
            .ok()?;
        watcher
            .watch(&root, notify::RecursiveMode::Recursive)
            .ok()?;
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Some(mut most) = changes.next().await {
                cx.background_executor().timer(SETTLE).await;
                while let Ok(more) = changes.try_recv() {
                    most = most.max(more);
                }
                let changed =
                    this.update_in(cx, |this, window, cx| this.repo_changed(most, window, cx));
                if changed.is_err() {
                    break;
                }
            }
        });
        Some(Self {
            root,
            _watcher: watcher,
            _task: task,
        })
    }
}

impl Workspace {
    pub(in crate::workspace) fn git_view_shown(&self) -> bool {
        self.sidebar_shown() && self.sidebar_view == SidebarView::Git
    }

    /// Watch the repository while the Git view shows, and stop when it
    /// doesn't. Call after anything that shows, hides or switches it.
    pub(in crate::workspace) fn sync_git_watch(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let wanted = self
            .git
            .panel
            .as_ref()
            .filter(|_| self.git_view_shown())
            .map(|panel| panel.view.read(cx).root().to_path_buf());
        if self.git.watch.as_ref().map(|watch| &watch.root) == wanted.as_ref() {
            return;
        }
        self.git.watch = None;
        self.git.watch = wanted.and_then(|root| RepoWatch::start(root, window, cx));
    }

    /// Something under the repository changed: the view's lists follow, and
    /// after a commit, stage or switch made elsewhere, the branch and the
    /// change bars too.
    pub(in crate::workspace) fn repo_changed(
        &mut self,
        touched: Touched,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.git_view_shown() {
            return;
        }
        match touched {
            Touched::Nothing => {}
            Touched::Files => self.refresh_git_panel(window, cx),
            Touched::Git => self.refresh_git(window, cx),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Touched, touched};
    use std::path::Path;

    #[test]
    fn only_git_files_that_matter_count() {
        let at = |path: &str| touched(Path::new(path));
        assert_eq!(at("/repo/src/main.rs"), Touched::Files);
        assert_eq!(at("/repo/target/debug/jig"), Touched::Files);
        assert_eq!(at("/repo/.git/HEAD"), Touched::Git);
        assert_eq!(at("/repo/.git/index"), Touched::Git);
        assert_eq!(at("/repo/.git/refs/heads/main"), Touched::Git);
        assert_eq!(at("/repo/.git/index.lock"), Touched::Nothing);
        assert_eq!(at("/repo/.git/objects/ab/cdef"), Touched::Nothing);
        assert_eq!(at("/repo/.git/logs/HEAD"), Touched::Nothing);
    }
}
