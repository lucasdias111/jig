//! Go to File (⌘P): a Spotlight-style panel that fuzzy-finds a file in the
//! project by name or path.
//!
//! With nothing typed it lists the files opened recently, then the rest of
//! the project. The project is walked in the background each time the panel
//! opens; the last walk's list shows meanwhile, so it opens instantly.

use std::collections::HashSet;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use ignore::WalkBuilder;

use crate::fuzzy;

pub const WIDTH: f32 = 560.;
const ROW_HEIGHT: f32 = 34.;
const MAX_VISIBLE_ROWS: usize = 10;
/// Rows listed at most; more matches than this are of no use to anyone.
const MAX_ROWS: usize = 200;
/// Walk no further than this many files, so a home folder opened as a
/// project can't stall the panel.
const MAX_FILES: usize = 200_000;

const SEARCH: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/></svg>"#;
const FILE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linejoin="round"><path d="M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z"/><path d="M14 3v5h5"/></svg>"#;
const CLOCK: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/></svg>"#;

pub enum QuickOpenEvent {
    /// A file was chosen.
    Open(PathBuf),
    /// A fresh walk of the project finished; worth keeping for next time.
    Indexed(Arc<Vec<String>>),
    /// Esc: close and go back to the editor.
    Dismissed,
    /// Focus moved elsewhere: close and leave it there.
    Blurred,
}

/// A listed file.
#[derive(Clone, Debug, PartialEq)]
struct Row {
    /// Relative to the project root, `/`-separated.
    path: SharedString,
    /// Byte offsets of the characters the query matched.
    positions: Vec<usize>,
    recent: bool,
}

pub struct QuickOpen {
    root: PathBuf,
    input: Entity<InputState>,
    /// Every file in the project, relative to `root`.
    files: Arc<Vec<String>>,
    indexing: bool,
    /// Enter was pressed before the first walk listed anything: open the
    /// best match once it does.
    open_when_indexed: bool,
    /// Files opened recently, most recent first, relative to `root`.
    recent: Vec<String>,
    rows: Vec<Row>,
    /// The query `rows` were built from.
    filtered_query: String,
    selected: usize,
    scroll: UniformListScrollHandle,
    _subscription: Subscription,
    _walk: Task<()>,
}

impl EventEmitter<QuickOpenEvent> for QuickOpen {}

impl Focusable for QuickOpen {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl QuickOpen {
    /// `files` is the last walk of `root`, if any; `recent` are absolute
    /// paths, most recent first.
    pub fn new(
        root: PathBuf,
        files: Option<Arc<Vec<String>>>,
        recent: &[PathBuf],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Go to file…"));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, _, cx| match event {
            InputEvent::Change => this.refilter(cx),
            InputEvent::Blur => cx.emit(QuickOpenEvent::Blurred),
            _ => {}
        });
        input.update(cx, |input, cx| input.focus(window, cx));

        let walk_root = root.clone();
        let walk = cx.spawn(async move |this, cx| {
            let files = cx
                .background_executor()
                .spawn(async move { walk(&walk_root) })
                .await;
            this.update(cx, |this, cx| {
                this.files = Arc::new(files);
                this.indexing = false;
                cx.emit(QuickOpenEvent::Indexed(this.files.clone()));
                this.refilter(cx);
                if this.open_when_indexed {
                    this.choose(0, cx);
                }
            })
            .ok();
        });

        let recent = recent
            .iter()
            .filter_map(|path| relative_path(&root, path))
            .collect();
        let mut this = Self {
            root,
            input,
            files: files.unwrap_or_default(),
            indexing: true,
            open_when_indexed: false,
            recent,
            rows: Vec::new(),
            filtered_query: String::new(),
            selected: 0,
            scroll: UniformListScrollHandle::default(),
            _subscription: subscription,
            _walk: walk,
        };
        this.refilter(cx);
        this
    }

    #[cfg(test)]
    pub fn row_paths(&self) -> Vec<String> {
        self.rows.iter().map(|row| row.path.to_string()).collect()
    }

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.rows = if query.trim().is_empty() {
            self.recent_then_all()
        } else {
            let recent: HashSet<&str> = self.recent.iter().map(String::as_str).collect();
            fuzzy::match_paths(self.files.iter().map(String::as_str), &query, MAX_ROWS)
                .into_iter()
                .map(|m| {
                    let path = &self.files[m.index];
                    Row {
                        recent: recent.contains(path.as_str()),
                        path: path.clone().into(),
                        positions: m.positions,
                    }
                })
                .collect()
        };
        self.filtered_query = query;
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
    }

    fn recent_then_all(&self) -> Vec<Row> {
        let recent: HashSet<&str> = self.recent.iter().map(String::as_str).collect();
        let row = |path: &String, recent| Row {
            path: path.clone().into(),
            positions: Vec::new(),
            recent,
        };
        self.recent
            .iter()
            .map(|path| row(path, true))
            .chain(
                self.files
                    .iter()
                    .filter(|path| !recent.contains(path.as_str()))
                    .map(|path| row(path, false)),
            )
            .take(MAX_ROWS)
            .collect()
    }

    /// Text typed in the same frame as Enter hasn't been filtered yet; do
    /// it now so the right row opens.
    fn ensure_filtered(&mut self, cx: &mut Context<Self>) {
        if self.query(cx) != self.filtered_query {
            self.refilter(cx);
        }
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get(index) {
            cx.emit(QuickOpenEvent::Open(self.root.join(row.path.as_ref())));
        }
    }

    fn on_enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.ensure_filtered(cx);
        if self.rows.is_empty() && self.indexing {
            self.open_when_indexed = true;
        }
        self.choose(self.selected, cx);
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        cx.emit(QuickOpenEvent::Dismissed);
    }

    fn on_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if !self.rows.is_empty() {
            self.select((self.selected + self.rows.len() - 1) % self.rows.len(), cx);
        }
    }

    fn on_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if !self.rows.is_empty() {
            self.select((self.selected + 1) % self.rows.len(), cx);
        }
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row = &self.rows[index];
        let selected = index == self.selected;
        let path: &str = row.path.as_ref();
        let name_start = path.rfind('/').map_or(0, |slash| slash + 1);
        let (name, folder) = (&path[name_start..], &path[..name_start.saturating_sub(1)]);

        // Matched characters: accent-coloured, or bold on the accent fill.
        let mark = HighlightStyle {
            color: (!selected).then_some(theme.primary),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        let marks = |range: Range<usize>, shift: usize| -> Vec<(Range<usize>, HighlightStyle)> {
            row.positions
                .iter()
                .filter(|&&i| range.contains(&i))
                .map(|&i| {
                    let len = path[i..].chars().next().map_or(1, char::len_utf8);
                    (i - shift..i - shift + len, mark)
                })
                .collect()
        };
        let name_text = StyledText::new(SharedString::from(name.to_string()))
            .with_highlights(marks(name_start..path.len(), name_start));
        let folder_text = StyledText::new(SharedString::from(folder.to_string()))
            .with_highlights(marks(0..folder.len(), 0));

        let (text, muted) = if selected {
            (
                theme.primary_foreground,
                theme.primary_foreground.opacity(0.75),
            )
        } else {
            (theme.popover_foreground, theme.muted_foreground)
        };
        // Selected like a macOS menu item: accent fill, white text.
        h_flex()
            .id(("quick-open-row", index))
            .w_full()
            .overflow_hidden()
            .h(px(ROW_HEIGHT))
            .px_2p5()
            .gap_2p5()
            .rounded(px(7.))
            .text_color(text)
            .when(selected, |row| row.bg(theme.primary))
            .when(!selected, |row| {
                row.hover(|row| row.bg(theme.foreground.opacity(0.06)))
            })
            .child(
                Icon::default()
                    .data(FILE)
                    .size(px(16.))
                    .flex_none()
                    .text_color(if selected {
                        text
                    } else {
                        theme.primary.opacity(0.85)
                    }),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .items_baseline()
                    .child(
                        div()
                            .flex_none()
                            .max_w(relative(0.7))
                            .truncate()
                            .text_size(px(13.5))
                            .child(name_text),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(muted)
                            .child(folder_text),
                    ),
            )
            .when(selected, |this| {
                this.child(
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(muted)
                        .child("↩"),
                )
            })
            .when(row.recent && !selected, |this| {
                this.child(
                    Icon::default()
                        .data(CLOCK)
                        .size(px(12.))
                        .flex_none()
                        .text_color(muted.opacity(0.8)),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.choose(index, cx);
                }),
            )
            .into_any_element()
    }

    fn status(&self) -> String {
        let query = self.filtered_query.trim();
        if self.indexing && self.files.is_empty() {
            return "Indexing…".into();
        }
        let count = |n: usize, one: &str, many: &str| {
            let word = if n == 1 { one } else { many };
            let n = if n >= MAX_ROWS && !query.is_empty() {
                format!("{MAX_ROWS}+")
            } else {
                group_digits(n)
            };
            format!("{n} {word}")
        };
        if query.is_empty() {
            count(self.files.len(), "file", "files")
        } else {
            count(self.rows.len(), "match", "matches")
        }
    }
}

impl Render for QuickOpen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.rows.len();
        let visible = rows.min(MAX_VISIBLE_ROWS);
        let empty_query = self.filtered_query.trim().is_empty();
        let section = if empty_query && !self.recent.is_empty() {
            Some("Recently opened")
        } else {
            None
        };

        let list = uniform_list(
            "quick-open-rows",
            rows,
            cx.processor(|this, range: Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .w_full()
        .h(px(visible as f32 * ROW_HEIGHT));

        let empty = (rows == 0).then(|| {
            let message = if self.indexing && self.files.is_empty() {
                "Looking through the project…".to_string()
            } else if empty_query {
                "This project has no files yet.".to_string()
            } else {
                format!("No files match “{}”", self.filtered_query.trim())
            };
            v_flex()
                .items_center()
                .justify_center()
                .py_6()
                .gap_1()
                .child(
                    Icon::default()
                        .data(SEARCH)
                        .size(px(20.))
                        .text_color(theme.muted_foreground.opacity(0.6)),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme.muted_foreground)
                        .child(message),
                )
        });

        let divider = || {
            div()
                .h(px(1.))
                .mx_neg_1p5()
                .bg(theme.foreground.opacity(0.08))
        };

        let panel = jig_commands::surface::panel(cx)
            .flex()
            .flex_col()
            .key_context("JigQuickOpen")
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_up))
            .capture_action(cx.listener(Self::on_down))
            .w(px(WIDTH))
            .p_1p5()
            .child(
                // Spotlight-sized search field.
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
                    ),
            )
            .child(divider().mb_1())
            .when_some(section, |this, section| {
                this.child(
                    div()
                        .px_2p5()
                        .pt_1()
                        .pb_1()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.muted_foreground)
                        .child(section),
                )
            })
            .children(empty)
            .when(rows > 0, |this| this.child(list))
            .child(divider().mt_1())
            .child(
                h_flex()
                    .px_2p5()
                    .pt_1p5()
                    .pb_0p5()
                    .justify_between()
                    .child(jig_commands::surface::hint(self.status(), cx))
                    .child(jig_commands::surface::hint(
                        "↑↓ navigate · ↩ open · esc close",
                        cx,
                    )),
            );
        jig_commands::motion::pop_in(panel, "jig-quick-open")
    }
}

/// `path` relative to `root` with `/` separators, if it is inside it.
fn relative_path(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let parts: Vec<_> = rest
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// Every file under `root` that `.gitignore` doesn't exclude, sorted by
/// path, relative to `root`.
pub fn walk(root: &Path) -> Vec<String> {
    let mut walk = WalkBuilder::new(root);
    walk.hidden(false)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(true)
        .require_git(false)
        .parents(true)
        .sort_by_file_name(|a, b| a.cmp(b))
        .filter_entry(|entry| {
            let name = entry.file_name();
            name != ".git" && name != ".DS_Store"
        });
    walk.build()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_some_and(|t| t.is_file()))
        .filter_map(|entry| relative_path(root, entry.path()))
        .take(MAX_FILES)
        .collect()
}

/// `12345` as `12,345`.
pub fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{group_digits, walk};

    #[test]
    fn walk_skips_ignored_files_and_git() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::write(root.join(".git/HEAD"), "").unwrap();
        fs::create_dir_all(root.join("src/ui")).unwrap();
        fs::create_dir_all(root.join("target/debug")).unwrap();
        fs::write(root.join(".gitignore"), "target/\n*.log\n").unwrap();
        fs::write(root.join("src/ui/button.rs"), "").unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
        fs::write(root.join("target/debug/jig"), "").unwrap();
        fs::write(root.join("build.log"), "").unwrap();
        fs::write(root.join(".env"), "").unwrap();
        fs::write(root.join(".DS_Store"), "").unwrap();

        assert_eq!(
            walk(root),
            [".env", ".gitignore", "src/lib.rs", "src/ui/button.rs"]
        );
    }

    #[test]
    fn digits_are_grouped() {
        assert_eq!(group_digits(7), "7");
        assert_eq!(group_digits(1204), "1,204");
        assert_eq!(group_digits(200000), "200,000");
    }
}
