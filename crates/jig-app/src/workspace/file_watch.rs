//! Open files that change on disk outside Jig (another editor, a formatter,
//! git in a terminal) show the new text. The folders of open files are
//! watched; a tab with unsaved changes keeps them.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::*;
use jig_editor::EditorHandle;
use notify::Watcher as _;

use super::Workspace;
use super::tabs::canonical;

/// Saves often arrive as several events (write, rename, metadata); wait
/// this long for them to settle before reading the file.
const SETTLE: Duration = Duration::from_millis(100);

#[derive(Default)]
pub(super) struct FileWatch {
    watcher: Option<notify::RecommendedWatcher>,
    folders: HashSet<PathBuf>,
    _task: Option<Task<()>>,
}

impl FileWatch {
    /// Start watching, reloading `workspace`'s tabs as their files change.
    /// Off in tests, whose own writes would otherwise race them.
    pub(super) fn start(window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        if cfg!(test) {
            return Self::default();
        }
        let (sender, mut changes) = mpsc::unbounded::<Vec<PathBuf>>();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            // Reading a file is an event too on Linux; only changes count.
            if let Ok(event) = event
                && !event.kind.is_access()
            {
                let _ = sender.unbounded_send(event.paths);
            }
        });
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Some(paths) = changes.next().await {
                let mut changed: HashSet<PathBuf> = paths.into_iter().collect();
                cx.background_executor().timer(SETTLE).await;
                while let Ok(paths) = changes.try_recv() {
                    changed.extend(paths);
                }
                let reloaded = this.update_in(cx, |this, window, cx| {
                    this.reload_changed_files(Some(&changed), window, cx)
                });
                if reloaded.is_err() {
                    break;
                }
            }
        });
        Self {
            watcher: watcher.ok(),
            folders: HashSet::new(),
            _task: Some(task),
        }
    }

    /// Watch the folder `path` is in, if it isn't already.
    pub(super) fn watch(&mut self, path: &Path) {
        let Some(watcher) = self.watcher.as_mut() else {
            return;
        };
        let Some(folder) = canonical(path).parent().map(Path::to_path_buf) else {
            return;
        };
        if !self.folders.contains(&folder)
            && watcher
                .watch(&folder, notify::RecursiveMode::NonRecursive)
                .is_ok()
        {
            self.folders.insert(folder);
        }
    }
}

impl Workspace {
    /// Show what's on disk in each open tab whose file changed there and
    /// that has no unsaved changes, as one undo step that keeps the cursor.
    /// `only` limits it to those files; `None` checks every tab.
    pub(super) fn reload_changed_files(
        &mut self,
        only: Option<&HashSet<PathBuf>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reviewing = self.run.as_ref().is_some_and(|run| run.preview.is_some());
        for ix in 0..self.tabs.len() {
            let tab = &self.tabs[ix];
            if tab.dirty || (ix == self.active && reviewing) {
                continue;
            }
            let Some(path) = tab.document.path.as_deref() else {
                continue;
            };
            if only.is_some_and(|only| !only.contains(&canonical(path))) {
                continue;
            }
            // Deleted or unreadable: the tab keeps what it had.
            let Ok(disk) = std::fs::read_to_string(path) else {
                continue;
            };
            if disk == tab.document.saved_text {
                continue;
            }
            let text = tab.editor.text(cx);
            // Updated first, so the edit below leaves the tab clean.
            self.tabs[ix].document.saved_text = disk.clone();
            if text == disk {
                continue;
            }
            let editor = self.tabs[ix].editor.clone();
            let selection = editor.selection(cx);
            let (range, replacement) = crate::diff::changed_range(&text, &disk);
            let new_len = replacement.len();
            editor.apply_edit(range.clone(), replacement, window, cx);
            let follow = |offset| shift(offset, &range, new_len);
            editor.select(follow(selection.start)..follow(selection.end), cx);
        }
        cx.notify();
    }
}

/// Where `offset` ends up after `range` is replaced with `new_len` bytes:
/// unmoved before it, moved along after it, and at its start inside it.
fn shift(offset: usize, range: &Range<usize>, new_len: usize) -> usize {
    if offset <= range.start {
        offset
    } else if offset >= range.end {
        offset - range.end + range.start + new_len
    } else {
        range.start
    }
}

#[cfg(test)]
mod tests {
    use super::shift;

    #[test]
    fn offsets_follow_the_edit() {
        // "one two three" with "two" (4..7) replaced by "2".
        assert_eq!(shift(2, &(4..7), 1), 2);
        assert_eq!(shift(4, &(4..7), 1), 4);
        assert_eq!(shift(5, &(4..7), 1), 4);
        assert_eq!(shift(7, &(4..7), 1), 5);
        assert_eq!(shift(13, &(4..7), 1), 11);
    }
}
