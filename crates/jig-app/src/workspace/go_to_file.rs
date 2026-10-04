//! Go to File (⌘P): opening the quick-open panel and acting on its choice.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::*;
use jig_editor::EditorHandle;

use super::{GoToFile, TITLE_BAR_HEIGHT, Workspace};
use crate::quick_open::{QuickOpen, QuickOpenEvent};

/// Files remembered for the empty query.
const MAX_RECENT_FILES: usize = 30;

pub(super) struct OpenQuickOpen {
    pub(super) view: Entity<QuickOpen>,
    _events: Subscription,
}

/// The last walk of a project, shown while the next one runs.
pub(super) struct FileIndex {
    root: PathBuf,
    files: Arc<Vec<String>>,
}

impl Workspace {
    pub(super) fn go_to_file(&mut self, _: &GoToFile, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_open.is_some() || self.new_command.is_some() {
            return;
        }
        let Some(root) = self.project_root(cx) else {
            if !self.home {
                self.show_note(
                    "Open a folder to find its files with ⌘P.".into(),
                    window,
                    cx,
                );
            }
            return;
        };
        self.palette = None;
        self.find_in_files = None;
        let cached = self.cached_files(&root);
        let current = self
            .document()
            .path
            .clone()
            .map(|path| super::tabs::canonical(&path));
        let recent: Vec<PathBuf> = self
            .recent_files
            .iter()
            .filter(|path| Some(*path) != current.as_ref())
            .cloned()
            .collect();
        let view = cx.new(|cx| QuickOpen::new(root.clone(), cached, &recent, window, cx));
        let events = cx.subscribe_in(
            &view,
            window,
            move |this, _, event, window, cx| match event {
                QuickOpenEvent::Open(path) => {
                    this.close_quick_open(cx);
                    this.open_file(path, window, cx);
                }
                QuickOpenEvent::Indexed(files) => this.remember_files(root.clone(), files.clone()),
                QuickOpenEvent::Dismissed => {
                    this.close_quick_open(cx);
                    if this.home {
                        this.home_focus.focus(window, cx);
                    } else {
                        this.editor().focus(window, cx);
                    }
                }
                QuickOpenEvent::Blurred => this.close_quick_open(cx),
            },
        );
        self.quick_open = Some(OpenQuickOpen {
            view,
            _events: events,
        });
        cx.notify();
    }

    /// The folder shown in the sidebar, which Go to File and Find in Files
    /// look through.
    pub(super) fn project_root(&self, cx: &App) -> Option<PathBuf> {
        self.tree
            .as_ref()
            .map(|tree| tree.view.read(cx).root().to_path_buf())
    }

    /// The last walk of `root`, if it is the project last walked.
    pub(super) fn cached_files(&self, root: &Path) -> Option<Arc<Vec<String>>> {
        self.file_index
            .as_ref()
            .filter(|index| index.root == root)
            .map(|index| index.files.clone())
    }

    pub(super) fn remember_files(&mut self, root: PathBuf, files: Arc<Vec<String>>) {
        self.file_index = Some(FileIndex { root, files });
    }

    pub(super) fn close_quick_open(&mut self, cx: &mut Context<Self>) {
        if self.quick_open.take().is_some() {
            cx.notify();
        }
    }

    /// Put `path` first among the recently opened files.
    pub(super) fn note_recent_file(&mut self, path: &Path) {
        let path = super::tabs::canonical(path);
        self.recent_files.retain(|recent| *recent != path);
        self.recent_files.insert(0, path);
        self.recent_files.truncate(MAX_RECENT_FILES);
    }

    /// The panel, centred near the top of the window like Spotlight.
    pub(super) fn render_quick_open(&self, window: &Window) -> Option<AnyElement> {
        let open = self.quick_open.as_ref()?;
        let width = px(crate::quick_open::WIDTH);
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
}
