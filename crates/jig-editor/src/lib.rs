//! The editor adapter.
//!
//! The command layer talks to the text editor only through [`EditorHandle`].
//! Today it is backed by GPUI Kit's code editor; swapping in Jig's own editor
//! element later means writing another implementation of this trait.

use std::ops::Range;

use gpui_kit::component::input::{
    EditorState, RangeDecoration, RangeDecorationCollection, RangeDecorationStyle, Undo,
};
use gpui_kit::{App, Bounds, Entity, Hsla, Pixels, Point, Window, point};

pub trait EditorHandle {
    fn text(&self, cx: &App) -> String;
    fn language(&self, cx: &App) -> String;
    /// The selected range as UTF-8 byte offsets. Empty when nothing is selected.
    fn selection(&self, cx: &App) -> Range<usize>;
    fn cursor(&self, cx: &App) -> usize;
    /// Where floating UI should anchor: just below the selection (or the
    /// cursor), in window coordinates. `None` when the target is scrolled out
    /// of view or not laid out yet.
    fn anchor_point(&self, cx: &App) -> Option<Point<Pixels>>;
    /// Replace `range` with `text` as a single undo step. Returns the range the
    /// new text occupies.
    fn apply_edit(
        &self,
        range: Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Range<usize>;
    fn set_readonly(&self, readonly: bool, cx: &mut App);
    fn highlight(&self, ranges: Vec<(Range<usize>, Hsla)>, cx: &mut App);
    fn clear_highlights(&self, cx: &mut App);
    fn focus(&self, window: &mut Window, cx: &mut App);
    /// Undo the last edit, as Cmd+Z would. Does nothing while read-only.
    fn undo(&self, window: &mut Window, cx: &mut App);
}

/// [`EditorHandle`] backed by GPUI Kit's `EditorState`.
#[derive(Clone)]
pub struct KitEditor {
    state: Entity<EditorState>,
    highlights: RangeDecorationCollection,
}

impl KitEditor {
    pub fn new(state: Entity<EditorState>, cx: &mut App) -> Self {
        let highlights = state.update(cx, |state, cx| {
            state.create_range_decorations_collection(Vec::new(), cx)
        });
        Self { state, highlights }
    }

    pub fn state(&self) -> &Entity<EditorState> {
        &self.state
    }
}

impl EditorHandle for KitEditor {
    fn text(&self, cx: &App) -> String {
        self.state.read(cx).value().to_string()
    }

    fn language(&self, cx: &App) -> String {
        self.state.read(cx).language_name().to_string()
    }

    fn selection(&self, cx: &App) -> Range<usize> {
        self.state.read(cx).selected_range()
    }

    fn cursor(&self, cx: &App) -> usize {
        self.state.read(cx).cursor()
    }

    fn anchor_point(&self, cx: &App) -> Option<Point<Pixels>> {
        let state = self.state.read(cx);
        let selection = state.selected_range();
        let start = state.range_to_bounds(&(selection.start..selection.start))?;
        let end = state.range_to_bounds(&(selection.end..selection.end))?;
        Some(point(start.left(), end.bottom()))
    }

    fn apply_edit(
        &self,
        range: Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Range<usize> {
        let text = text.to_string();
        self.state.update(cx, |state, cx| {
            state.set_selected_range(range.clone(), cx);
            let start = state.selected_range().start;
            state.replace(text.clone(), window, cx);
            start..start + text.len()
        })
    }

    fn set_readonly(&self, readonly: bool, cx: &mut App) {
        self.state
            .update(cx, |state, cx| state.set_readonly(readonly, cx));
    }

    fn highlight(&self, ranges: Vec<(Range<usize>, Hsla)>, cx: &mut App) {
        let decorations = ranges
            .into_iter()
            .map(|(range, color)| {
                RangeDecoration::new(range)
                    .with_style(RangeDecorationStyle::Fill)
                    .with_color(color)
            })
            .collect();
        self.highlights.set(decorations, cx);
    }

    fn clear_highlights(&self, cx: &mut App) {
        self.highlights.clear(cx);
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        self.state.update(cx, |state, cx| state.focus(window, cx));
    }

    fn undo(&self, window: &mut Window, cx: &mut App) {
        self.focus(window, cx);
        window.dispatch_action(Box::new(Undo), cx);
    }
}

/// The selection's bounds, for debugging the anchor position.
pub fn selection_bounds(editor: &KitEditor, cx: &App) -> Option<Bounds<Pixels>> {
    let state = editor.state.read(cx);
    state.range_to_bounds(&state.selected_range())
}
