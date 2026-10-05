//! Tabs: one open file each, with its own buffer, unsaved state and undo
//! history, and the strip that shows them.

use std::path::Path;

use gpui_kit::component::input::{EditorState, InputEvent, TabSize};
use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_editor::{EditorHandle, KitEditor};

use super::status_bar::CursorPosition;
use super::{ActivateTab, CloseTab, NewFile, NextTab, PreviousTab, Workspace};
use crate::document::Document;

const CLOSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"#;

/// Tabs share the title bar's width between these bounds; past the
/// narrowest, the strip scrolls.
/// Height of the row of tabs under the title bar.
const TAB_BAR_HEIGHT: f32 = 36.;
const MIN_TAB_WIDTH: f32 = 120.;
const MAX_TAB_WIDTH: f32 = 260.;

pub(super) struct Tab {
    pub(super) document: Document,
    pub(super) editor: KitEditor,
    pub(super) dirty: bool,
    /// Detected from the file when it was opened; what Tab inserts.
    pub(super) indentation: TabSize,
    pub(super) cursor: Entity<CursorPosition>,
    /// The file open in its language server, if it has one.
    pub(super) lsp: Option<super::lsp::TabLsp>,
    /// Snippets just offered in the completion list.
    pub(super) offers: super::snippets::Offers,
    /// The snippet being filled in, if any.
    pub(super) snippet: Option<super::snippets::Session>,
    /// Breakpoints, and the line the debugger is paused on.
    pub(super) marks: super::breakpoints::Marks,
    _events: Subscription,
}

impl Tab {
    /// Identifies the tab across reordering, e.g. while a prompt is open.
    pub(super) fn id(&self) -> EntityId {
        self.editor.state().entity_id()
    }

    /// An Untitled tab nobody has typed into, which opening a file reuses.
    pub(super) fn is_blank(&self, cx: &App) -> bool {
        self.document.path.is_none() && !self.dirty && self.editor.text(cx).is_empty()
    }
}

pub(super) fn canonical(path: &Path) -> std::path::PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

impl Workspace {
    pub(super) fn tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub(super) fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub(super) fn editor(&self) -> &KitEditor {
        &self.tab().editor
    }

    pub(super) fn document(&self) -> &Document {
        &self.tab().document
    }

    pub(super) fn any_dirty(&self) -> bool {
        self.tabs.iter().any(|tab| tab.dirty)
    }

    pub(super) fn new_tab(
        &mut self,
        document: Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Tab {
        let editor = &self.settings.editor;
        if let Some(path) = document.path.as_deref() {
            self.file_watch.watch(path);
        }
        let indentation = crate::indentation::detect(&document.saved_text);
        let state = cx.new(|cx| {
            EditorState::new(window, cx)
                // Jig's own find panel takes ⌘F, over the same search.
                .searchable(false)
                .language(document.language(&self.settings.languages))
                .tab_size(indentation)
                .line_number(editor.line_numbers)
                .soft_wrap(editor.soft_wrap)
                .indent_guides(editor.indent_guides)
                .show_whitespaces(editor.show_whitespace)
                .default_value(document.saved_text.clone())
        });
        super::definitions::install(&state, cx);
        let offers = super::completions::install(&state, cx);
        let events = cx.subscribe_in(
            &state,
            window,
            |this, state, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::Change)
                    && let Some(ix) = this.tabs.iter().position(|tab| tab.editor.state() == state)
                {
                    this.refresh_dirty(ix, window, cx);
                    this.lsp_changed(ix, cx);
                    this.snippet_changed(ix, cx);
                    this.git_buffer_changed(ix, cx);
                    if ix == this.active {
                        this.on_buffer_changed(cx);
                    }
                }
                cx.notify();
            },
        );
        let cursor = cx.new(|cx| CursorPosition::new(state.clone(), cx));
        let marks = self.install_marks(&state, &document, cx);
        self.install_git(&state, &document, cx);
        let lsp = document.path.as_deref().and_then(|path| {
            let language = document.language(&self.settings.languages);
            super::lsp::TabLsp::open(path, language, &document.saved_text, cx)
        });
        Tab {
            document,
            editor: KitEditor::new(state, cx),
            dirty: false,
            indentation,
            cursor,
            lsp,
            offers,
            snippet: None,
            marks,
            _events: events,
        }
    }

    fn refresh_dirty(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tab = &mut self.tabs[ix];
        let dirty = tab.document.is_dirty(&tab.editor.text(cx));
        if dirty != tab.dirty {
            tab.dirty = dirty;
            self.update_title(window);
        }
    }

    pub(super) fn tab_for(&self, path: &Path) -> Option<usize> {
        let path = canonical(path);
        self.tabs.iter().position(|tab| {
            tab.document
                .path
                .as_deref()
                .is_some_and(|open| canonical(open) == path)
        })
    }

    /// Show `path`: switch to its tab if it is open, otherwise open it in a
    /// new tab next to the current one.
    pub fn open_file(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.tab_for(path) {
            self.activate(ix, window, cx);
            return;
        }
        let document = match Document::open(path) {
            Ok(document) => document,
            Err(error) => return self.show_error(&format!("{error:#}"), window, cx),
        };
        self.leave_home(cx);
        let tab = self.new_tab(document, window, cx);
        self.leave_tab(cx);
        if self.tab().is_blank(cx) {
            self.tabs[self.active] = tab;
        } else {
            self.active += 1;
            self.tabs.insert(self.active, tab);
        }
        self.enter_tab(window, cx);
    }

    /// Open `path` in a new last tab without switching to it, for edits
    /// made in files you aren't looking at. Returns its index.
    pub(super) fn open_in_background(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<usize> {
        let document = Document::open(path).ok()?;
        let tab = self.new_tab(document, window, cx);
        self.tabs.push(tab);
        cx.notify();
        Some(self.tabs.len() - 1)
    }

    /// Re-open the current tab's file from disk, e.g. so highlighting follows
    /// a new extension after Save As.
    pub(super) fn reload_tab(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        match Document::open(path) {
            Ok(document) => {
                self.remember_breakpoints(self.active, cx);
                let tab = self.new_tab(document, window, cx);
                self.leave_tab(cx);
                self.tabs[self.active] = tab;
                self.enter_tab(window, cx);
            }
            Err(error) => self.show_error(&format!("{error:#}"), window, cx),
        }
    }

    pub(super) fn activate(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        if ix != self.active {
            self.leave_tab(cx);
            self.active = ix;
        }
        self.enter_tab(window, cx);
    }

    /// Settle anything floating over the current tab before it goes out of
    /// view: a change under review is kept, a running command is cancelled.
    pub(super) fn leave_tab(&mut self, cx: &mut Context<Self>) {
        self.palette = None;
        self.close_find_panel(cx);
        self.accept_preview(cx);
        if self.run.take().is_some() {
            self.editor().clear_highlights(cx);
        }
    }

    fn enter_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor().focus(window, cx);
        self.tab_scroll.scroll_to_item(self.active);
        self.update_title(window);
        if let Some(path) = self.document().path.clone() {
            self.note_recent_file(&path);
            self.show_in_tree(&path, window, cx);
        }
        cx.notify();
    }

    pub(super) fn new_file(&mut self, _: &NewFile, window: &mut Window, cx: &mut Context<Self>) {
        if self.home {
            // The blank tab behind the home page is the new file.
            self.leave_home(cx);
            self.enter_tab(window, cx);
            return;
        }
        let tab = self.new_tab(Document::default(), window, cx);
        self.leave_tab(cx);
        self.active += 1;
        self.tabs.insert(self.active, tab);
        self.enter_tab(window, cx);
    }

    pub(super) fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab_at(self.active, window, cx);
    }

    /// Close the tab at `ix`, asking first if it has unsaved changes.
    /// Closing the last tab shows the start page; closing that closes the
    /// window.
    fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        if self.home {
            window.remove_window();
            return;
        }
        let id = tab.id();
        self.when_discard_ok(Some(&[id]), window, cx, move |this, window, cx| {
            this.remove_tab(id, window, cx)
        });
    }

    /// Close every tab, or with `keep`, every tab but that one, asking once
    /// first if any of them has unsaved changes.
    pub(super) fn close_tabs_except(
        &mut self,
        keep: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let keep = keep.and_then(|ix| self.tabs.get(ix)).map(Tab::id);
        let ids: Vec<EntityId> = self
            .tabs
            .iter()
            .map(Tab::id)
            .filter(|id| Some(*id) != keep)
            .collect();
        if ids.is_empty() {
            return;
        }
        self.when_discard_ok(Some(&ids.clone()), window, cx, move |this, window, cx| {
            for id in ids {
                this.remove_tab(id, window, cx);
            }
            if let Some(ix) = keep.and_then(|id| this.tabs.iter().position(|tab| tab.id() == id)) {
                this.activate(ix, window, cx);
            }
        });
    }

    pub(super) fn remove_tab(&mut self, id: EntityId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.tabs.iter().position(|tab| tab.id() == id) else {
            return;
        };
        if ix == self.active {
            self.leave_tab(cx);
        }
        self.remember_breakpoints(ix, cx);
        self.tabs.remove(ix);
        if self.tabs.is_empty() {
            let tab = self.new_tab(Document::default(), window, cx);
            self.tabs.push(tab);
            self.home = true;
            if self.tree.is_some() {
                self.focus_tree(window, cx);
            } else {
                self.focus_main(window, cx);
            }
            self.update_title(window);
            cx.notify();
            return;
        }
        // The tab to the right takes the closed one's place, or the one to
        // the left when it was last.
        if ix < self.active || self.active >= self.tabs.len() {
            self.active -= 1;
        }
        self.enter_tab(window, cx);
    }

    pub(super) fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        self.activate((self.active + 1) % self.tabs.len(), window, cx);
    }

    pub(super) fn previous_tab(
        &mut self,
        _: &PreviousTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let len = self.tabs.len();
        self.activate((self.active + len - 1) % len, window, cx);
    }

    /// Cmd+1…8 pick that tab; Cmd+9 always picks the last, as in browsers.
    pub(super) fn activate_tab(
        &mut self,
        action: &ActivateTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ix = if action.0 == 8 {
            self.tabs.len() - 1
        } else {
            action.0
        };
        self.activate(ix, window, cx);
    }

    /// Each tab's label: the file name, plus its folder when another tab
    /// has the same name.
    pub(super) fn tab_labels(&self) -> Vec<String> {
        let titles: Vec<String> = self.tabs.iter().map(|tab| tab.document.title()).collect();
        self.tabs
            .iter()
            .zip(&titles)
            .map(|(tab, title)| {
                let shared = titles.iter().filter(|other| *other == title).count() > 1;
                let parent = tab
                    .document
                    .path
                    .as_deref()
                    .and_then(Path::parent)
                    .and_then(Path::file_name);
                match parent {
                    Some(parent) if shared => format!("{title} — {}", parent.to_string_lossy()),
                    _ => title.clone(),
                }
            })
            .collect()
    }

    /// What the title bar shows: the project's name, or nothing outside a
    /// project. Either way it fills the space, keeping the controls right.
    pub(super) fn render_title(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let title = self.project_root(cx).and_then(|root| {
            root.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
        h_flex()
            .flex_1()
            .min_w_0()
            .justify_center()
            .gap_1()
            .text_size(px(13.))
            .children(title.map(|title| {
                div()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground.opacity(0.85))
                    .child(title)
            }))
            .into_any_element()
    }

    /// The open tabs, in a row above the editor, even when there's only
    /// one. None on the home page.
    pub(super) fn render_tab_bar(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.home {
            return None;
        }
        let theme = cx.theme();
        let languages = crate::settings::get(cx).languages;
        let workspace = cx.entity().downgrade();
        let count = self.tabs.len();
        let tabs = self
            .tabs
            .iter()
            .zip(self.tab_labels())
            .enumerate()
            .map(|(ix, (tab, label))| {
                let active = ix == self.active;
                let group = SharedString::from(format!("tab-{ix}"));
                // The close button sits on the left, as in Safari, on the
                // hovered tab; an unsaved tab shows a dot there until hovered.
                let close = div()
                    .id(("close-tab", ix))
                    .absolute()
                    .inset_0()
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(theme.foreground.opacity(0.1)))
                    .child(
                        Icon::default()
                            .data(CLOSE)
                            .size(px(9.))
                            .text_color(theme.muted_foreground),
                    )
                    .invisible()
                    .group_hover(group.clone(), |s| s.visible())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_tab_at(ix, window, cx);
                    }));
                let marker = tab.dirty.then(|| {
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .group_hover(group.clone(), |s| s.invisible())
                        .child(div().size(px(6.)).rounded_full().bg(theme.muted_foreground))
                });
                h_flex()
                    .id(("tab", ix))
                    .group(group.clone())
                    .relative()
                    .flex_1()
                    .flex_basis(px(0.))
                    .min_w(px(MIN_TAB_WIDTH))
                    .max_w(px(MAX_TAB_WIDTH))
                    .h(px(28.))
                    .px_1p5()
                    .gap_1()
                    .rounded(px(7.))
                    .text_size(px(12.))
                    .when(active, |this| {
                        this.bg(theme.foreground.opacity(0.07))
                            .text_color(theme.foreground)
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!active, |this| {
                        this.text_color(theme.muted_foreground)
                            .hover(|s| s.bg(theme.foreground.opacity(0.04)))
                    })
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .size(px(16.))
                            .children(marker)
                            .child(close),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .justify_center()
                            .items_center()
                            .gap_1p5()
                            .children(
                                tab.document
                                    .path
                                    .as_deref()
                                    .and_then(|path| {
                                        crate::file_icons::for_path(
                                            path,
                                            &languages,
                                            theme.is_dark(),
                                        )
                                    })
                                    .map(|(svg, color)| {
                                        Icon::default()
                                            .data(svg)
                                            .size(px(11.))
                                            .flex_none()
                                            .text_color(if active {
                                                color
                                            } else {
                                                color.opacity(0.7)
                                            })
                                    }),
                            )
                            .child(div().truncate().child(label)),
                    )
                    // Balances the close button so the name stays centred.
                    .child(div().flex_none().size(px(16.)))
                    // Clicking a tab selects it rather than dragging the window.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| this.activate(ix, window, cx)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| this.close_tab_at(ix, window, cx)),
                    )
                    .context_menu({
                        let workspace = workspace.clone();
                        move |menu, _, _| tab_menu(workspace.clone(), ix, count, menu)
                    })
            });
        Some(
            h_flex()
                .id("tab-bar")
                .flex_none()
                .w_full()
                .h(px(TAB_BAR_HEIGHT))
                .px_2()
                .gap_1()
                .border_b_1()
                .border_color(theme.title_bar_border)
                .overflow_x_scroll()
                .track_scroll(&self.tab_scroll)
                .children(tabs)
                .into_any_element(),
        )
    }
}

/// Right-clicking a tab offers to close it, the others, or all of them.
fn tab_menu(
    workspace: WeakEntity<Workspace>,
    ix: usize,
    tabs: usize,
    menu: PopupMenu,
) -> PopupMenu {
    let item = |label: &'static str, keep: Option<Option<usize>>| {
        let workspace = workspace.clone();
        PopupMenuItem::new(label).on_click(move |_, window, cx| {
            workspace
                .update(cx, |this, cx| match keep {
                    None => this.close_tab_at(ix, window, cx),
                    Some(keep) => this.close_tabs_except(keep, window, cx),
                })
                .ok();
        })
    };
    menu.item(item("Close Tab", None))
        .item(item("Close Other Tabs", Some(Some(ix))).disabled(tabs < 2))
        .item(item("Close All Tabs", Some(None)))
}
