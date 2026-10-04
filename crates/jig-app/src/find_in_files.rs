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

use crate::languages::language_for;
use crate::project_search::{self, LineMatch, MAX_RESULTS, SearchOptions, SearchResults};
use crate::quick_open::group_digits;
use crate::settings::LanguageSettings;

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

/// The selected match's file, shown read-only below the list.
struct Preview {
    path: String,
    editor: KitEditor,
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
                .spawn(async move { crate::quick_open::walk(&walk_root) })
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

    #[cfg(test)]
    pub fn result_lines(&self) -> Vec<String> {
        self.results
            .matches
            .iter()
            .map(|m| format!("{}:{}: {}", m.path, m.line + 1, m.text))
            .collect()
    }

    #[cfg(test)]
    pub fn is_searching(&self) -> bool {
        self.searching
    }

    fn preview_focused(&self, window: &Window, cx: &App) -> bool {
        self.preview.as_ref().is_some_and(|preview| {
            preview
                .editor
                .state()
                .focus_handle(cx)
                .contains_focused(window, cx)
        })
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
        let pattern = match project_search::pattern(&query, options) {
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
                .spawn(async move { project_search::search(&root, &files, &pattern, &cancel) })
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

    /// Show the selected match in the preview, loading its file if it
    /// isn't the one already shown.
    fn update_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(found) = self.results.matches.get(self.selected).cloned() else {
            self.preview = None;
            return;
        };
        if self.preview.as_ref().is_none_or(|p| p.path != found.path) {
            let full = self.root.join(&found.path);
            let Ok(text) = std::fs::read_to_string(&full) else {
                self.preview = None;
                return;
            };
            let language = language_for(&full, &self.languages);
            let state = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language(language)
                    .line_number(true)
                    .default_value(text)
            });
            let editor = KitEditor::new(state, cx);
            editor.set_readonly(true, cx);
            self.preview = Some(Preview {
                path: found.path.clone(),
                editor,
            });
        }
        if let Some(preview) = &self.preview {
            let highlight = cx.theme().primary.opacity(0.28);
            preview.editor.select(found.range.clone(), cx);
            preview.editor.highlight(vec![(found.range, highlight)], cx);
        }
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

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let found: &LineMatch = &self.results.matches[index];
        let selected = index == self.selected;
        let name = found
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&found.path)
            .to_string();
        let mark = HighlightStyle {
            background_color: Some(if selected {
                theme.primary_foreground.opacity(0.25)
            } else {
                theme.primary.opacity(0.22)
            }),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        };
        let line = StyledText::new(SharedString::from(found.text.clone()))
            .with_highlights(found.highlights.iter().map(|range| (range.clone(), mark)));
        let (text, muted) = if selected {
            (
                theme.primary_foreground,
                theme.primary_foreground.opacity(0.75),
            )
        } else {
            (theme.popover_foreground, theme.muted_foreground)
        };
        h_flex()
            .id(("find-in-files-row", index))
            // Held to the list's width, so a long line truncates instead of
            // pushing the file name out of view.
            .w_full()
            .overflow_hidden()
            .h(px(ROW_HEIGHT))
            .px_2p5()
            .gap_4()
            .rounded(px(6.))
            .text_color(text)
            .when(selected, |row| row.bg(theme.primary))
            .when(!selected, |row| {
                row.hover(|row| row.bg(theme.foreground.opacity(0.06)))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(px(12.5))
                    .child(line),
            )
            .child(
                h_flex()
                    .id(("find-in-files-file", index))
                    .flex_none()
                    .flex_shrink_0()
                    .max_w(px(260.))
                    .gap_1p5()
                    .text_size(px(12.))
                    .child(div().min_w_0().truncate().text_color(muted).child(name))
                    .child(
                        div()
                            .flex_none()
                            .text_color(muted.opacity(0.8))
                            .child((found.line + 1).to_string()),
                    )
                    .test_support(),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.choose(index, cx);
                } else {
                    this.select(index, window, cx);
                    this.input.update(cx, |input, cx| input.focus(window, cx));
                }
            }))
            .test_support()
            .into_any_element()
    }

    /// An option chip, IntelliJ style: `Cc`, `W`, `.*`.
    fn render_option(
        &self,
        id: &'static str,
        label: &'static str,
        tooltip: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        toggle: impl Fn(&mut SearchOptions) + 'static,
    ) -> AnyElement {
        let theme = cx.theme();
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .h(px(24.))
            .min_w(px(28.))
            .px_1p5()
            .rounded(px(6.))
            .text_size(px(12.))
            .font_family(theme.mono_font_family.clone())
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .when(on, |this| {
                this.bg(theme.primary.opacity(0.16))
                    .text_color(theme.primary)
                    .border_1()
                    .border_color(theme.primary.opacity(0.45))
            })
            .when(!on, |this| {
                this.text_color(theme.muted_foreground)
                    .border_1()
                    .border_color(transparent_black())
                    .hover(|s| {
                        s.bg(theme.foreground.opacity(0.06))
                            .text_color(theme.foreground)
                    })
            })
            .child(label)
            .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
            // Keep the search field focused.
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle(window, cx, &toggle);
            }))
            .into_any_element()
    }

    fn status(&self) -> String {
        if let Some(error) = &self.error {
            return error.clone();
        }
        if self.searching {
            return "Searching…".into();
        }
        if self.searched.0.is_empty() {
            return match &self.files {
                Some(files) => format!("{} files", group_digits(files.len())),
                None => "Indexing…".into(),
            };
        }
        let lines = self.results.total_lines;
        let files = self.results.files;
        let plural =
            |n: usize, one: &'static str, many: &'static str| if n == 1 { one } else { many };
        let mut status = format!(
            "{} {} in {} {}",
            group_digits(lines),
            plural(lines, "match", "matches"),
            group_digits(files),
            plural(files, "file", "files"),
        );
        if lines > MAX_RESULTS {
            status.push_str(&format!(
                " · showing the first {}",
                group_digits(MAX_RESULTS)
            ));
        }
        status
    }
}

impl Render for FindInFiles {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let options = self.options;
        let match_case = self.render_option(
            "opt-case",
            "Cc",
            "Match Case (⌥C)",
            options.match_case,
            cx,
            |o| o.match_case = !o.match_case,
        );
        let whole_word = self.render_option(
            "opt-word",
            "W",
            "Words (⌥W)",
            options.whole_word,
            cx,
            |o| o.whole_word = !o.whole_word,
        );
        let regex = self.render_option("opt-regex", ".*", "Regex (⌥X)", options.regex, cx, |o| {
            o.regex = !o.regex
        });

        let theme = cx.theme();
        let rows = self.results.matches.len();
        let visible = rows.min(MAX_VISIBLE_ROWS);
        let list = uniform_list(
            "find-in-files-rows",
            rows,
            cx.processor(|this, range: Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .w_full()
        .h(px(visible as f32 * ROW_HEIGHT));

        let empty = (rows == 0 && !self.searching && self.error.is_none()).then(|| {
            let query = self.searched.0.clone();
            let message = if query.is_empty() {
                "Type to search every file in the project".to_string()
            } else {
                format!("Nothing found for “{query}”")
            };
            v_flex()
                .items_center()
                .justify_center()
                .py_8()
                .gap_1p5()
                .child(
                    Icon::default()
                        .data(SEARCH)
                        .size(px(22.))
                        .text_color(theme.muted_foreground.opacity(0.5)),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme.muted_foreground)
                        .child(message),
                )
        });

        let preview = self.preview.as_ref().filter(|_| rows > 0).map(|preview| {
            let line = self
                .results
                .matches
                .get(self.selected)
                .map_or(0, |found| found.line + 1);
            v_flex()
                .mx_neg_1p5()
                .mb_neg_1p5()
                .border_t_1()
                .border_color(theme.foreground.opacity(0.08))
                .child(
                    h_flex()
                        .px_4()
                        .py_1p5()
                        .gap_1()
                        .text_size(px(11.5))
                        .text_color(theme.muted_foreground)
                        .bg(theme.foreground.opacity(0.03))
                        .child(div().truncate().child(preview.path.clone()))
                        .child(div().flex_none().opacity(0.7).child(format!(":{line}"))),
                )
                .child(
                    div()
                        .h(px(PREVIEW_HEIGHT))
                        .bg(theme.background)
                        .rounded_b(px(jig_commands::surface::RADIUS))
                        .pl_1()
                        .pt_1()
                        .child(
                            gpui_kit::component::input::Editor::new(preview.editor.state())
                                .bordered(false)
                                .size_full(),
                        ),
                )
        });

        let divider = || {
            div()
                .h(px(1.))
                .mx_neg_1p5()
                .bg(theme.foreground.opacity(0.08))
        };
        let status_color = if self.error.is_some() {
            theme.danger
        } else {
            theme.muted_foreground
        };

        let panel = jig_commands::surface::panel(cx)
            .flex()
            .flex_col()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::toggle_match_case))
            .on_action(cx.listener(Self::toggle_whole_word))
            .on_action(cx.listener(Self::toggle_regex))
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_up))
            .capture_action(cx.listener(Self::on_down))
            .w(px(WIDTH))
            .p_1p5()
            .child(
                h_flex()
                    .px_2p5()
                    .pt_1()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .justify_between()
                    .child("Find in Files")
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(status_color)
                            .child(self.status()),
                    ),
            )
            .child(
                h_flex()
                    .h(px(40.))
                    .pl_2p5()
                    .pr_1()
                    .gap_1()
                    .child(
                        Icon::default()
                            .data(SEARCH)
                            .size(px(17.))
                            .flex_none()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(16.))
                            .child(Input::new(&self.input).appearance(false).cleanable(false)),
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_0p5()
                            .child(match_case)
                            .child(whole_word)
                            .child(regex),
                    ),
            )
            .child(divider().mb_1())
            .children(empty)
            .when(rows > 0, |this| this.child(list).child(div().h_1()))
            .children(preview)
            .when(self.preview.is_none() || rows == 0, |this| {
                this.child(divider().mt_1()).child(
                    h_flex().px_2p5().pt_1p5().pb_0p5().justify_end().child(
                        jig_commands::surface::hint(
                            "↑↓ navigate · ↩ open · ⌥C ⌥W ⌥X options · esc close",
                            cx,
                        ),
                    ),
                )
            });
        jig_commands::motion::pop_in(panel, "jig-find-in-files")
    }
}
