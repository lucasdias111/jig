//! Find in Files (⇧⌘F): opening the panel and going to the chosen match.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::*;
use jig_editor::EditorHandle;

use super::{FindInFiles, TITLE_BAR_HEIGHT, Workspace};
use jig_search::find_in_files::{self, FindInFilesEvent};
use jig_search::project_search::SearchOptions;

/// A selection longer than this isn't taken as the query.
const MAX_SELECTION_QUERY: usize = 200;

/// The selection in `text`, when it's short and on one line: what Find
/// and Find in Files start looking for.
pub(super) fn selection_query(text: &str, selection: Range<usize>) -> Option<String> {
    let selected = text.get(selection)?;
    (!selected.is_empty() && !selected.contains('\n') && selected.len() <= MAX_SELECTION_QUERY)
        .then(|| selected.to_string())
}

/// Find in Files' panel, while it's open, and the last search. Its own
/// entity, so the workspace only opens it, places it and goes where it
/// says.
#[derive(Default)]
pub(super) struct FindInFilesPanel {
    open: Option<OpenFindInFiles>,
    last: LastFind,
}

struct OpenFindInFiles {
    view: Entity<find_in_files::FindInFiles>,
    _events: Subscription,
}

/// The last search, to start the next one from, as IntelliJ does.
#[derive(Default)]
struct LastFind {
    query: String,
    options: SearchOptions,
}

/// What the panel asks of the workspace, which has the code and the focus.
pub(super) enum FindInFilesPanelEvent {
    /// Go to `range` in `path`. The panel has closed.
    Open { path: PathBuf, range: Range<usize> },
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

impl EventEmitter<FindInFilesPanelEvent> for FindInFilesPanel {}

impl FindInFilesPanel {
    /// The panel, while it's open.
    pub(super) fn panel(&self) -> Option<&Entity<find_in_files::FindInFiles>> {
        self.open.as_ref().map(|open| &open.view)
    }

    pub(super) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The last search's query and options.
    fn last(&self) -> (String, SearchOptions) {
        (self.last.query.clone(), self.last.options)
    }

    /// Open the panel on `root`, searching for `query`, with the files
    /// `cached` from the last walk to start from.
    #[allow(clippy::too_many_arguments)]
    fn open(
        &mut self,
        root: PathBuf,
        cached: Option<Arc<Vec<String>>>,
        query: String,
        options: SearchOptions,
        languages: jig_settings::LanguageSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.new(|cx| {
            find_in_files::FindInFiles::new(
                root.clone(),
                cached,
                &query,
                options,
                languages,
                window,
                cx,
            )
        });
        let events = cx.subscribe(&view, move |this, view, event, cx| {
            if !matches!(event, FindInFilesEvent::Indexed(_)) {
                let view = view.read(cx);
                this.last = LastFind {
                    query: view.query(cx),
                    options: view.options(),
                };
            }
            let event = match event {
                FindInFilesEvent::Open { path, range } => FindInFilesPanelEvent::Open {
                    path: path.clone(),
                    range: range.clone(),
                },
                FindInFilesEvent::Indexed(files) => {
                    cx.emit(FindInFilesPanelEvent::Indexed {
                        root: root.clone(),
                        files: files.clone(),
                    });
                    return;
                }
                FindInFilesEvent::Dismissed => FindInFilesPanelEvent::Dismissed,
                FindInFilesEvent::Blurred => FindInFilesPanelEvent::Blurred,
            };
            this.close(cx);
            cx.emit(event);
        });
        self.open = Some(OpenFindInFiles {
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
}

impl Render for FindInFilesPanel {
    /// The panel, centred near the top of the window.
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let Some(open) = &self.open else {
            return div().into_any_element();
        };
        let width = px(find_in_files::WIDTH);
        let left = ((window.viewport_size().width - width) / 2.).max(px(8.));
        deferred(
            anchored()
                .position(point(left, px(TITLE_BAR_HEIGHT + 16.)))
                .snap_to_window_with_margin(px(8.))
                .child(open.view.clone()),
        )
        .into_any_element()
    }
}

impl Workspace {
    pub(super) fn find_in_files(
        &mut self,
        _: &FindInFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.find_in_files.read(cx).panel().cloned() {
            // Already open: back to its search field.
            window.focus(&view.focus_handle(cx), cx);
            return;
        }
        if self.project_root(cx).is_none() {
            if !self.home {
                self.show_note(
                    "Open a folder to search its files with ⇧⌘F.".into(),
                    window,
                    cx,
                );
            }
            return;
        }

        // A short selection on one line is what to look for; otherwise
        // start from the last search.
        let selected = if self.home {
            None
        } else {
            selection_query(&self.editor().text(cx), self.editor().selection(cx))
        };
        let (last_query, options) = self.find_in_files.read(cx).last();
        let query = selected.unwrap_or(last_query);
        self.open_find_in_files(query, options, window, cx);
    }

    /// Open the panel searching for `query`. Does nothing without a
    /// project.
    pub(super) fn open_find_in_files(
        &mut self,
        query: String,
        options: SearchOptions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.project_root(cx) else {
            return;
        };
        self.palette = None;
        self.close_quick_open(cx);
        let cached = self.cached_files(&root);
        let languages = self.settings.languages.clone();
        self.find_in_files.update(cx, |panel, cx| {
            panel.open(root, cached, query, options, languages, window, cx)
        });
        cx.notify();
    }

    /// Act on what the panel says: go to a match, keep a walk of the
    /// project, or back to the code.
    pub(super) fn on_find_in_files_event(
        &mut self,
        _: &Entity<FindInFilesPanel>,
        event: &FindInFilesPanelEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            FindInFilesPanelEvent::Open { path, range } => {
                cx.notify();
                self.remember_location(cx);
                self.open_file(path, window, cx);
                if self.document().path.as_deref().is_some_and(|open| {
                    super::tabs::canonical(open) == super::tabs::canonical(path)
                }) {
                    self.editor().select(range.clone(), cx);
                    self.focus_editor(window, cx);
                }
            }
            FindInFilesPanelEvent::Indexed { root, files } => {
                self.remember_files(root.clone(), files.clone());
            }
            FindInFilesPanelEvent::Dismissed => {
                cx.notify();
                if self.home {
                    self.home_focus.focus(window, cx);
                } else {
                    self.focus_editor(window, cx);
                }
            }
            FindInFilesPanelEvent::Blurred => cx.notify(),
        }
    }

    pub(super) fn close_find_in_files(&mut self, cx: &mut Context<Self>) {
        if self.find_in_files.update(cx, |panel, cx| panel.close(cx)) {
            cx.notify();
        }
    }

    /// The panel, while it's open.
    pub(super) fn render_find_in_files(&self, cx: &App) -> Option<AnyElement> {
        self.find_in_files
            .read(cx)
            .is_open()
            .then(|| self.find_in_files.clone().into_any_element())
    }
}
