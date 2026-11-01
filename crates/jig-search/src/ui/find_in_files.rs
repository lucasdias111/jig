//! Find in Files (⇧⌘F), as in IntelliJ: a panel that searches the text of
//! every file in the project, lists each matching line, and previews the
//! chosen one in context.
//!
//! The search reruns as you type, in the background; a new keystroke
//! cancels the search still running. ↩ opens the file with the match
//! selected.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::base::TestSupportExt as _;
use gpui_kit::component::input::{
    EditorState, Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp,
};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_editor::{EditorHandle as _, KitEditor};

use crate::domain::files::group_digits;
use crate::domain::text_search::{self, LineMatch, MAX_RESULTS, SearchOptions, SearchResults};
use crate::store::persist::{self, ProjectFolder};
use jig_languages::language_for;
use jig_settings::LanguageSettings;

pub const WIDTH: f32 = 760.;
const ROW_HEIGHT: f32 = 26.;
const MAX_VISIBLE_ROWS: usize = 11;
const PREVIEW_HEIGHT: f32 = 230.;
const CONTEXT: &str = "JigFindInFiles";

const SEARCH: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>"#;

actions!(
    find_in_files,
    [ToggleMatchCase, ToggleWholeWord, ToggleRegex]
);

/// IntelliJ's shortcuts for the three options.
pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("alt-c", ToggleMatchCase, Some(CONTEXT)),
        KeyBinding::new("alt-w", ToggleWholeWord, Some(CONTEXT)),
        KeyBinding::new("alt-x", ToggleRegex, Some(CONTEXT)),
    ]
}

pub enum FindInFilesEvent {
    /// Open `path` with `range` selected.
    Open { path: PathBuf, range: Range<usize> },
    /// A fresh walk of the project finished; worth keeping for next time.
    Indexed(Arc<Vec<String>>),
    /// Esc: close and go back to the editor.
    Dismissed,
    /// Focus left the panel: close and leave focus where it went.
    Blurred,
}

pub struct FindInFiles {
    root: PathBuf,
    input: Entity<InputState>,
    options: SearchOptions,
    /// Every file in the project; `None` until the first walk finishes.
    files: Option<Arc<Vec<String>>>,
    languages: LanguageSettings,
    results: SearchResults,
    /// The query and options `results` are for.
    searched: (String, SearchOptions),
    searching: bool,
    /// The results were handed in (e.g. a symbol's references) rather than
    /// searched for; they stay until the query or options change.
    pinned: bool,
    /// Enter was pressed while the search ran: open its first match.
    open_when_done: bool,
    /// Why the query can't be searched, e.g. a broken regex.
    error: Option<String>,
    selected: usize,
    scroll: UniformListScrollHandle,
    preview: Option<Preview>,
    /// Set to stop the search in flight when a new one starts.
    cancel: Arc<AtomicBool>,
    _search: Option<Task<()>>,
    _walk: Option<Task<()>>,
    _subscription: Subscription,
}

impl EventEmitter<FindInFilesEvent> for FindInFiles {}

impl Focusable for FindInFiles {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl FindInFiles {
    /// `files` is the last walk of `root`, if any. The panel starts with
    /// `query` typed and selected, so typing replaces it.
    pub fn new(
        root: PathBuf,
        files: Option<Arc<Vec<String>>>,
        query: &str,
        options: SearchOptions,
        languages: LanguageSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Find in files…")
                .default_value(query.to_string())
        });
        let subscription =
            cx.subscribe_in(&input, window, |this, _, event, window, cx| match event {
                InputEvent::Change => {
                    // A new query cancels an Enter waiting on the old one.
                    this.open_when_done = false;
                    this.start_search(window, cx)
                }
                // Clicking into the preview keeps the panel open.
                InputEvent::Blur if !this.preview_focused(window, cx) => {
                    cx.emit(FindInFilesEvent::Blurred)
                }
                _ => {}
            });
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });

        // Walk afresh every time; a cached list searches meanwhile.
        let walk_root = root.clone();
        let walk = cx.spawn_in(window, async move |this, cx| {
            let files = cx
                .background_executor()
                .spawn(async move { jig_commands::project_files::walk(&walk_root) })
                .await;
            this.update_in(cx, |this, window, cx| {
                let files = Arc::new(files);
                let first = this.files.is_none();
                this.files = Some(files.clone());
                cx.emit(FindInFilesEvent::Indexed(files));
                if first && !this.pinned {
                    this.start_search(window, cx);
                }
            })
            .ok();
        });

        let mut this = Self {
            root,
            input,
            options,
            files,
            languages,
            results: SearchResults::default(),
            searched: (String::new(), options),
            searching: false,
            pinned: false,
            open_when_done: false,
            error: None,
            selected: 0,
            scroll: UniformListScrollHandle::default(),
            preview: None,
            cancel: Arc::new(AtomicBool::new(false)),
            _search: None,
            _walk: Some(walk),
            _subscription: subscription,
        };
        this.start_search(window, cx);
        this
    }

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    pub fn options(&self) -> SearchOptions {
        self.options
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn result_lines(&self) -> Vec<String> {
        self.results
            .matches
            .iter()
            .map(|m| format!("{}:{}: {}", m.path, m.line + 1, m.text))
            .collect()
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn is_searching(&self) -> bool {
        self.searching
    }

    /// Show `results` for the current query instead of searching for it.
    pub fn show_results(
        &mut self,
        results: SearchResults,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel.store(true, Ordering::Relaxed);
        self._search = None;
        self.searching = false;
        self.pinned = true;
        self.error = None;
        let searched = (self.query(cx), self.options);
        self.set_results(results, searched, window, cx);
    }

    fn start_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.pinned = false;
        self.cancel.store(true, Ordering::Relaxed);
        self._search = None;
        let query = self.query(cx);
        let options = self.options;
        self.error = None;
        if query.is_empty() {
            self.set_results(SearchResults::default(), (query, options), window, cx);
            self.searching = false;
            return;
        }
        let pattern = match text_search::pattern(&query, options) {
            Ok(pattern) => pattern,
            Err(error) => {
                self.error = Some(error);
                self.searching = false;
                cx.notify();
                return;
            }
        };
        self.searching = true;
        let Some(files) = self.files.clone() else {
            // Starts again once the walk is done.
            cx.notify();
            return;
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.cancel = cancel.clone();
        let root = self.root.clone();
        self._search = Some(cx.spawn_in(window, async move |this, cx| {
            let results = cx
                .background_executor()
                .spawn(async move {
                    text_search::search(&ProjectFolder::new(&root), &files, &pattern, &cancel)
                })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.searching = false;
                this.set_results(results, (query, options), window, cx);
                if std::mem::take(&mut this.open_when_done) {
                    this.choose(0, cx);
                }
            })
            .ok();
        }));
        cx.notify();
    }

    fn set_results(
        &mut self,
        results: SearchResults,
        searched: (String, SearchOptions),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.results = results;
        self.searched = searched;
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        self.update_preview(window, cx);
        cx.notify();
    }

    fn toggle(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut SearchOptions),
    ) {
        f(&mut self.options);
        self.open_when_done = false;
        self.start_search(window, cx);
    }

    fn toggle_match_case(
        &mut self,
        _: &ToggleMatchCase,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle(window, cx, |o| o.match_case = !o.match_case);
    }

    fn toggle_whole_word(
        &mut self,
        _: &ToggleWholeWord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle(window, cx, |o| o.whole_word = !o.whole_word);
    }

    fn toggle_regex(&mut self, _: &ToggleRegex, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle(window, cx, |o| o.regex = !o.regex);
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(found) = self.results.matches.get(index) {
            cx.emit(FindInFilesEvent::Open {
                path: self.root.join(&found.path),
                range: found.range.clone(),
            });
        }
    }

    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = index;
        self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        self.update_preview(window, cx);
        cx.notify();
    }

    /// The list's keys work from the search field; in the preview, the
    /// arrows and Enter are the editor's.
    fn input_focused(&self, window: &Window, cx: &App) -> bool {
        self.input.focus_handle(cx).is_focused(window)
    }

    fn on_enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        if !self.input_focused(window, cx) {
            return;
        }
        cx.stop_propagation();
        // Results for an older query would open the wrong place.
        if self.searched == (self.query(cx), self.options) {
            self.choose(self.selected, cx);
        } else if self.searching {
            self.open_when_done = true;
        }
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        cx.emit(FindInFilesEvent::Dismissed);
    }

    fn on_up(&mut self, _: &MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.results.matches.len();
        if self.input_focused(window, cx) && rows > 0 {
            cx.stop_propagation();
            self.select((self.selected + rows - 1) % rows, window, cx);
        }
    }

    fn on_down(&mut self, _: &MoveDown, window: &mut Window, cx: &mut Context<Self>) {
        let rows = self.results.matches.len();
        if self.input_focused(window, cx) && rows > 0 {
            cx.stop_propagation();
            self.select((self.selected + 1) % rows, window, cx);
        }
    }
}

mod preview;
mod view;

use preview::Preview;
