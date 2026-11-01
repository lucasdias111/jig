//! Go to File (⌘P): a Spotlight-style panel that fuzzy-finds a file in the
//! project by name or path.
//!
//! With nothing typed it lists the files opened recently, then the rest of
//! the project. The project is walked in the background each time the panel
//! opens; the last walk's list shows meanwhile, so it opens instantly.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use jig_commands::project_files::relative_path;

use crate::domain::files::{self, Row, group_digits};
use crate::store::persist::walk;

// Asked of the workspace from elsewhere, such as the file tree's menu.
actions!(jig, [GoToFile]);

pub const WIDTH: f32 = 560.;
const ROW_HEIGHT: f32 = 34.;
const MAX_VISIBLE_ROWS: usize = 10;
/// Rows listed at most; more matches than this are of no use to anyone.
const MAX_ROWS: usize = 200;

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

    #[cfg(any(test, feature = "testing"))]
    pub fn row_paths(&self) -> Vec<String> {
        self.rows.iter().map(|row| row.path.to_string()).collect()
    }

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.rows = files::rows(&self.files, &self.recent, &query, MAX_ROWS);
        self.filtered_query = query;
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        cx.notify();
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
            cx.emit(QuickOpenEvent::Open(self.root.join(&row.path)));
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
}

mod view;
