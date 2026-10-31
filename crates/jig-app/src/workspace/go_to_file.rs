//! Go to File (⌘P): opening the quick-open panel and acting on its choice.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::*;

use super::{GoToFile, TITLE_BAR_HEIGHT, Workspace};
use jig_search::quick_open::{QuickOpen, QuickOpenEvent};

/// Files remembered for the empty query.
const MAX_RECENT_FILES: usize = 30;

/// Go to File's panel, while it's open, and the files shown in this window
/// for it to offer first. Its own entity, so the workspace only opens it,
/// places it and opens what it says.
#[derive(Default)]
pub(super) struct GoToFilePanel {
    open: Option<OpenQuickOpen>,
    /// Files shown in this window, most recent first.
    recent: Vec<PathBuf>,
}

struct OpenQuickOpen {
    view: Entity<QuickOpen>,
    _events: Subscription,
}

/// What the panel asks of the workspace, which opens files and has the
/// focus.
pub(super) enum GoToFileEvent {
    /// Open `path`. The panel has closed.
    Open(PathBuf),
    /// `root`'s files were walked, for the next search to start from.
    Indexed {
        root: PathBuf,
        files: Arc<Vec<String>>,
    },
    /// The panel was dismissed and has closed: back to the code.
    Dismissed,
    /// The panel lost the keyboard and has closed.
    Blurred,
}

impl EventEmitter<GoToFileEvent> for GoToFilePanel {}

impl GoToFilePanel {
    /// The panel, while it's open.
    #[cfg(test)]
    pub(super) fn panel(&self) -> Option<&Entity<QuickOpen>> {
        self.open.as_ref().map(|open| &open.view)
    }

    pub(super) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Open the panel on `root`, with the files `cached` from the last walk
    /// to start from, offering recent files other than `current` first.
    fn open(
        &mut self,
        root: PathBuf,
        cached: Option<Arc<Vec<String>>>,
        current: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let recent: Vec<PathBuf> = self
            .recent
            .iter()
            .filter(|path| Some(*path) != current.as_ref())
            .cloned()
            .collect();
        let view = cx.new(|cx| QuickOpen::new(root.clone(), cached, &recent, window, cx));
        let events = cx.subscribe(&view, move |this, _, event, cx| {
            let event = match event {
                QuickOpenEvent::Open(path) => GoToFileEvent::Open(path.clone()),
                QuickOpenEvent::Indexed(files) => {
                    cx.emit(GoToFileEvent::Indexed {
                        root: root.clone(),
                        files: files.clone(),
                    });
                    return;
                }
                QuickOpenEvent::Dismissed => GoToFileEvent::Dismissed,
                QuickOpenEvent::Blurred => GoToFileEvent::Blurred,
            };
            this.close(cx);
            cx.emit(event);
        });
        self.open = Some(OpenQuickOpen {
            view,
            _events: events,
        });
        cx.notify();
    }

    /// Close the panel. Returns whether it was open.
    pub(super) fn close(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.open.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    /// Put `path` first among the recently shown files.
    fn note_recent(&mut self, path: PathBuf) {
        self.recent.retain(|recent| *recent != path);
        self.recent.insert(0, path);
        self.recent.truncate(MAX_RECENT_FILES);
    }

    /// Forget everything, for another project.
    pub(super) fn reset(&mut self, cx: &mut Context<Self>) {
        *self = Self::default();
        cx.notify();
    }
}

impl Render for GoToFilePanel {
    /// The panel, centred near the top of the window like Spotlight.
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let Some(open) = &self.open else {
            return div().into_any_element();
        };
        let width = px(jig_search::quick_open::WIDTH);
        let left = ((window.viewport_size().width - width) / 2.).max(px(8.));
        deferred(
            anchored()
                .position(point(left, px(TITLE_BAR_HEIGHT + 24.)))
                .snap_to_window_with_margin(px(8.))
                .child(open.view.clone()),
        )
        .into_any_element()
    }
}

/// The last walk of a project, shown while the next one runs. Go to File,
/// Find in Files and Go to Definition's guesses share it.
pub(super) struct FileIndex {
    root: PathBuf,
    files: Arc<Vec<String>>,
}

impl Workspace {
    pub(super) fn go_to_file(&mut self, _: &GoToFile, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_open.read(cx).is_open() {
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
        self.close_find_in_files(cx);
        let cached = self.cached_files(&root);
        let current = self
            .document()
            .path
            .clone()
            .map(|path| super::tabs::canonical(&path));
        self.quick_open.update(cx, |panel, cx| {
            panel.open(root, cached, current, window, cx)
        });
        cx.notify();
    }

    /// Act on what the panel says: open a file, keep a walk of the project,
    /// or back to the code.
    pub(super) fn on_go_to_file_event(
        &mut self,
        _: &Entity<GoToFilePanel>,
        event: &GoToFileEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            GoToFileEvent::Open(path) => {
                cx.notify();
                self.open_file(path, window, cx);
            }
            GoToFileEvent::Indexed { root, files } => {
                self.remember_files(root.clone(), files.clone())
            }
            GoToFileEvent::Dismissed => {
                cx.notify();
                if self.home {
                    self.home_focus.focus(window, cx);
                } else {
                    self.focus_editor(window, cx);
                }
            }
            GoToFileEvent::Blurred => cx.notify(),
        }
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
        if self.quick_open.update(cx, |panel, cx| panel.close(cx)) {
            cx.notify();
        }
    }

    /// Put `path` first among the recently opened files.
    pub(super) fn note_recent_file(&mut self, path: &Path, cx: &mut Context<Self>) {
        let path = super::tabs::canonical(path);
        self.quick_open
            .update(cx, |panel, _| panel.note_recent(path));
    }

    /// The panel, while it's open.
    pub(super) fn render_quick_open(&self, cx: &App) -> Option<AnyElement> {
        self.quick_open
            .read(cx)
            .is_open()
            .then(|| self.quick_open.clone().into_any_element())
    }
}
