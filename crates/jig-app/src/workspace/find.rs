//! Find in Files (⇧⌘F): opening the panel and going to the chosen match.

use gpui_kit::*;
use jig_editor::EditorHandle;

use super::{FindInFiles, TITLE_BAR_HEIGHT, Workspace};
use crate::find_in_files::{self, FindInFilesEvent};
use crate::project_search::SearchOptions;

/// A selection longer than this isn't taken as the query.
const MAX_SELECTION_QUERY: usize = 200;

pub(super) struct OpenFindInFiles {
    pub(super) view: Entity<find_in_files::FindInFiles>,
    _events: Subscription,
}

/// The last search, to start the next one from, as IntelliJ does.
#[derive(Default)]
pub(super) struct LastFind {
    query: String,
    options: SearchOptions,
}

impl Workspace {
    pub(super) fn find_in_files(
        &mut self,
        _: &FindInFiles,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.new_command.is_some() {
            return;
        }
        if let Some(open) = &self.find_in_files {
            // Already open: back to its search field.
            let view = open.view.clone();
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
            String::new()
        } else {
            let range = self.editor().selection(cx);
            let text = self.editor().text(cx);
            text.get(range).unwrap_or_default().to_string()
        };
        let query = if !selected.is_empty()
            && !selected.contains('\n')
            && selected.len() <= MAX_SELECTION_QUERY
        {
            selected
        } else {
            self.last_find.query.clone()
        };
        let options = self.last_find.options;
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
        self.quick_open = None;
        let cached = self.cached_files(&root);
        let languages = self.settings.languages.clone();
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
        let events = cx.subscribe_in(&view, window, move |this, view, event, window, cx| {
            if !matches!(event, FindInFilesEvent::Indexed(_)) {
                let view = view.read(cx);
                this.last_find = LastFind {
                    query: view.query(cx),
                    options: view.options(),
                };
            }
            match event {
                FindInFilesEvent::Open { path, range } => {
                    this.close_find_in_files(cx);
                    this.open_file(path, window, cx);
                    if this.document().path.as_deref().is_some_and(|open| {
                        super::tabs::canonical(open) == super::tabs::canonical(path)
                    }) {
                        this.editor().select(range.clone(), cx);
                        this.editor().focus(window, cx);
                    }
                }
                FindInFilesEvent::Indexed(files) => {
                    this.remember_files(root.clone(), files.clone());
                }
                FindInFilesEvent::Dismissed => {
                    this.close_find_in_files(cx);
                    if this.home {
                        this.home_focus.focus(window, cx);
                    } else {
                        this.editor().focus(window, cx);
                    }
                }
                FindInFilesEvent::Blurred => this.close_find_in_files(cx),
            }
        });
        self.find_in_files = Some(OpenFindInFiles {
            view,
            _events: events,
        });
        cx.notify();
    }

    pub(super) fn close_find_in_files(&mut self, cx: &mut Context<Self>) {
        if self.find_in_files.take().is_some() {
            cx.notify();
        }
    }

    /// The panel, centred near the top of the window.
    pub(super) fn render_find_in_files(&self, window: &Window) -> Option<AnyElement> {
        let open = self.find_in_files.as_ref()?;
        let width = px(find_in_files::WIDTH);
        let left = ((window.viewport_size().width - width) / 2.).max(px(8.));
        Some(
            deferred(
                anchored()
                    .position(point(left, px(TITLE_BAR_HEIGHT + 16.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(open.view.clone()),
            )
            .into_any_element(),
        )
    }
}
