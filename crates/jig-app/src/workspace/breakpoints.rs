//! Breakpoints: toggled by clicking the gutter or with ⌘F8, shown as red
//! dots, and remembered across launches in `~/.config/jig/breakpoints.json`.
//! Only files whose language has its debugger turned on in Settings take
//! them; in others the breakpoint column isn't there.
//!
//! In an open file a breakpoint is a range decoration over its line, so it
//! moves with edits as the line does, and goes when the line is deleted.
//! Files that aren't open keep their lines in [`Breakpoints`].

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;
use std::path::{Path, PathBuf};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{
    EditorState, RangeDecoration, RangeDecorationCollection, RangeDecorationStyle,
};
use gpui_kit::*;
use jig_editor::EditorHandle as _;
use serde::{Deserialize, Serialize};

use super::tabs::canonical;
use super::{ToggleBreakpoint, Workspace};
use crate::document::Document;

/// A tab's breakpoints and the line the debugger is paused on.
pub(super) struct Marks {
    breakpoints: RangeDecorationCollection,
    execution: RangeDecorationCollection,
    /// The file takes breakpoints: its debugger is on.
    enabled: bool,
}

/// Breakpoints by file, 0-based lines, as last remembered.
#[derive(Default)]
pub(super) struct Breakpoints {
    by_file: BTreeMap<PathBuf, BTreeSet<u32>>,
    /// Where they're kept; `None` in tests.
    store: Option<PathBuf>,
}

#[derive(Default, Serialize, Deserialize)]
struct Stored {
    files: BTreeMap<PathBuf, BTreeSet<u32>>,
}

impl Breakpoints {
    pub(super) fn load() -> Self {
        if cfg!(test) {
            return Self::default();
        }
        let store = std::env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".config/jig/breakpoints.json"));
        let by_file = store
            .as_deref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| serde_json::from_str::<Stored>(&text).ok())
            .map(|stored| stored.files)
            .unwrap_or_default();
        Self { by_file, store }
    }

    fn save(&self) {
        let Some(store) = &self.store else {
            return;
        };
        let stored = Stored {
            files: self.by_file.clone(),
        };
        if let Some(dir) = store.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&stored) {
            let _ = std::fs::write(store, text);
        }
    }

    fn lines(&self, path: &Path) -> BTreeSet<u32> {
        self.by_file.get(path).cloned().unwrap_or_default()
    }

    fn set(&mut self, path: PathBuf, lines: BTreeSet<u32>) {
        let changed = if lines.is_empty() {
            self.by_file.remove(&path).is_some()
        } else {
            self.by_file.insert(path, lines.clone()) != Some(lines)
        };
        if changed {
            self.save();
        }
    }
}

impl Workspace {
    /// Breakpoint and execution-line decorations for a new tab's editor,
    /// with the file's remembered breakpoints and gutter clicks toggling
    /// them, if its language can be debugged.
    pub(super) fn install_marks(
        &self,
        state: &Entity<EditorState>,
        document: &Document,
        cx: &mut Context<Self>,
    ) -> Marks {
        let enabled = self.takes_breakpoints(document);
        let decorations = if enabled {
            self.remembered_decorations(document, &document.saved_text, cx)
        } else {
            Vec::new()
        };
        let handler = enabled.then(|| self.gutter_handler(state.entity_id(), cx));
        state.update(cx, |state, cx| {
            if let Some(handler) = handler {
                state.on_gutter_click(handler);
            }
            Marks {
                breakpoints: state.create_range_decorations_collection(decorations, cx),
                execution: state.create_range_decorations_collection(Vec::new(), cx),
                enabled,
            }
        })
    }

    /// Whether `document`'s language has its debugger turned on.
    fn takes_breakpoints(&self, document: &Document) -> bool {
        let language = document.language(&self.settings.languages);
        crate::debuggers::registry()
            .for_language(language)
            .is_some_and(|debugger| self.settings.debugging.is_enabled(&debugger.key))
    }

    fn remembered_decorations(
        &self,
        document: &Document,
        text: &str,
        cx: &App,
    ) -> Vec<RangeDecoration> {
        let lines = document
            .path
            .as_deref()
            .map(|path| self.breakpoints.lines(&canonical(path)))
            .unwrap_or_default();
        breakpoint_decorations(text, &lines, cx.theme().red)
    }

    /// What clicking tab `id`'s gutter does.
    fn gutter_handler(
        &self,
        id: EntityId,
        cx: &mut Context<Self>,
    ) -> impl Fn(usize, &mut Window, &mut App) + 'static {
        let workspace = cx.entity().downgrade();
        move |line, window, cx| {
            workspace
                .update(cx, |this, cx| {
                    this.toggle_breakpoint_line(id, line as u32, window, cx)
                })
                .ok();
        }
    }

    /// After Settings changed which debuggers are on: show or hide each
    /// tab's breakpoints and breakpoint column to match.
    pub(super) fn refresh_breakpoint_gutters(&mut self, cx: &mut Context<Self>) {
        for ix in 0..self.tabs.len() {
            let enabled = self.takes_breakpoints(&self.tabs[ix].document);
            if enabled == self.tabs[ix].marks.enabled {
                continue;
            }
            self.remember_breakpoints(ix, cx);
            let state = self.tabs[ix].editor.state().clone();
            if enabled {
                let text = self.tabs[ix].editor.text(cx);
                let decorations = self.remembered_decorations(&self.tabs[ix].document, &text, cx);
                let handler = self.gutter_handler(state.entity_id(), cx);
                state.update(cx, |state, cx| {
                    state.on_gutter_click(handler);
                    cx.notify();
                });
                self.tabs[ix].marks.breakpoints.set(decorations, cx);
            } else {
                state.update(cx, |state, cx| state.clear_gutter_click(cx));
                self.tabs[ix].marks.breakpoints.clear(cx);
            }
            self.tabs[ix].marks.enabled = enabled;
        }
    }

    /// ⌘F8: a breakpoint on the cursor's line, or none.
    pub(super) fn toggle_breakpoint(
        &mut self,
        _: &ToggleBreakpoint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home || !self.tab().marks.enabled {
            return;
        }
        let text = self.editor().text(cx);
        let line = line_of(&text, self.editor().cursor(cx));
        let id = self.tab().id();
        self.toggle_breakpoint_line(id, line, window, cx);
    }

    fn toggle_breakpoint_line(
        &mut self,
        id: EntityId,
        line: u32,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.tabs.iter().position(|tab| tab.id() == id) else {
            return;
        };
        let Some(path) = self.tabs[ix].document.path.as_deref().map(canonical) else {
            return;
        };
        let mut lines = self.breakpoint_lines(ix, cx);
        if !lines.remove(&line) {
            lines.insert(line);
        }
        let text = self.tabs[ix].editor.text(cx);
        let decorations = breakpoint_decorations(&text, &lines, cx.theme().red);
        self.tabs[ix].marks.breakpoints.set(decorations, cx);
        self.breakpoints
            .set(path.clone(), self.breakpoint_lines(ix, cx));
        self.breakpoints_changed(&path, cx);
        cx.notify();
    }

    /// The tab's breakpoints where its edits have moved them.
    pub(super) fn breakpoint_lines(&self, ix: usize, cx: &App) -> BTreeSet<u32> {
        let tab = &self.tabs[ix];
        let text = tab.editor.text(cx);
        tab.marks
            .breakpoints
            .get_ranges(cx)
            .into_iter()
            .map(|range| line_of(&text, range.start))
            .collect()
    }

    /// Keep the tab's breakpoints as they now are, e.g. before it closes.
    pub(super) fn remember_breakpoints(&mut self, ix: usize, cx: &App) {
        // A file whose debugger is off shows none, but keeps them.
        if !self.tabs[ix].marks.enabled {
            return;
        }
        if let Some(path) = self.tabs[ix].document.path.as_deref().map(canonical) {
            let lines = self.breakpoint_lines(ix, cx);
            self.breakpoints.set(path, lines);
        }
    }

    /// Every breakpoint, by file, open tabs as edited.
    pub(super) fn all_breakpoints(&mut self, cx: &App) -> Vec<(PathBuf, Vec<u32>)> {
        for ix in 0..self.tabs.len() {
            self.remember_breakpoints(ix, cx);
        }
        self.breakpoints
            .by_file
            .iter()
            .map(|(path, lines)| (path.clone(), lines.iter().copied().collect()))
            .collect()
    }

    /// Every breakpoint in files the debugger `key` debugs.
    pub(super) fn breakpoints_for(&mut self, key: &str, cx: &App) -> Vec<(PathBuf, Vec<u32>)> {
        let languages = self.settings.languages.clone();
        let registry = crate::debuggers::registry();
        self.all_breakpoints(cx)
            .into_iter()
            .filter(|(path, _)| {
                let language = crate::languages::language_for(path, &languages);
                registry
                    .for_language(language)
                    .is_some_and(|debugger| debugger.key == key)
            })
            .collect()
    }

    /// The breakpoints in `path`, open or not.
    pub(super) fn breakpoints_in(&self, path: &Path, cx: &App) -> Vec<u32> {
        match self.tab_for(path) {
            Some(ix) => self.breakpoint_lines(ix, cx).into_iter().collect(),
            None => self.breakpoints.lines(path).into_iter().collect(),
        }
    }

    /// Open `path` at `line` (0-based) and mark it as where the debugger is
    /// paused.
    pub(super) fn show_execution_line(
        &mut self,
        path: &Path,
        line: u32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_execution_line(cx);
        let position = lsp_types::Position::new(line, 0);
        self.open_at(path, lsp_types::Range::new(position, position), window, cx);
        let Some(ix) = self.tab_for(path) else {
            return;
        };
        let text = self.tabs[ix].editor.text(cx);
        if let Some(range) = line_range(&text, line) {
            let color = cx.theme().blue.opacity(0.22);
            self.tabs[ix].marks.execution.set(
                vec![
                    RangeDecoration::new(range)
                        .with_style(RangeDecorationStyle::Fill)
                        .with_color(color),
                ],
                cx,
            );
        }
    }

    /// Where the execution line is marked, if anywhere.
    #[cfg(test)]
    pub(super) fn execution_line(&self, cx: &App) -> Option<(PathBuf, u32)> {
        self.tabs.iter().find_map(|tab| {
            let range = tab.marks.execution.get_ranges(cx).into_iter().next()?;
            let line = line_of(&tab.editor.text(cx), range.start);
            Some((tab.document.path.clone()?, line))
        })
    }

    pub(super) fn clear_execution_line(&mut self, cx: &mut Context<Self>) {
        for tab in &self.tabs {
            tab.marks.execution.clear(cx);
        }
    }
}

fn breakpoint_decorations(text: &str, lines: &BTreeSet<u32>, color: Hsla) -> Vec<RangeDecoration> {
    lines
        .iter()
        .filter_map(|&line| line_range(text, line))
        .map(|range| {
            // Invisible over the text; the dot is in the gutter.
            RangeDecoration::new(range)
                .with_style(RangeDecorationStyle::Fill)
                .with_color(transparent_black())
                .with_gutter_marker(color)
        })
        .collect()
}

/// The 0-based line `offset` is on.
fn line_of(text: &str, offset: usize) -> u32 {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    text[..offset].matches('\n').count() as u32
}

/// Line `line`'s bytes, its newline included; `None` past the end or for an
/// empty last line, which has nothing to decorate.
fn line_range(text: &str, line: u32) -> Option<Range<usize>> {
    let mut start = 0;
    for _ in 0..line {
        start += text[start..].find('\n')? + 1;
    }
    let end = text[start..]
        .find('\n')
        .map_or(text.len(), |ix| start + ix + 1);
    (end > start).then_some(start..end)
}

#[cfg(test)]
mod tests {
    use super::{line_of, line_range};

    #[test]
    fn lines_and_their_ranges() {
        let text = "fn a() {}\n\nlet x = 1;";
        assert_eq!(line_range(text, 0), Some(0..10));
        assert_eq!(line_range(text, 1), Some(10..11));
        assert_eq!(line_range(text, 2), Some(11..21));
        assert_eq!(line_range(text, 3), None);
        assert_eq!(line_range("a\n", 1), None);
        assert_eq!(line_of(text, 12), 2);
        assert_eq!(line_of(text, 10), 1);
    }
}
