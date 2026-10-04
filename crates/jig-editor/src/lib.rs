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
    /// Just right of the code in `range`, in window coordinates: past the
    /// end of its longest line on screen, level with its first line on
    /// screen. `None` when none of it is on screen.
    fn beside_point(&self, range: Range<usize>, cx: &App) -> Option<Point<Pixels>>;
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
    /// Select `range` (UTF-8 byte offsets) and scroll it into view.
    fn select(&self, range: Range<usize>, cx: &mut App);
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

    /// Ranges currently highlighted through [`EditorHandle::highlight`].
    pub fn highlighted_ranges(&self, cx: &App) -> Vec<Range<usize>> {
        self.highlights.get_ranges(cx)
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

    fn beside_point(&self, range: Range<usize>, cx: &App) -> Option<Point<Pixels>> {
        let state = self.state.read(cx);
        let text = state.value();
        // Lines that end with the selection's last newline aren't in it.
        let last = if range.end > range.start && text[..range.end].ends_with('\n') {
            range.end - 1
        } else {
            range.end
        };
        let first_line = text[..range.start].rfind('\n').map_or(0, |i| i + 1);
        let mut beside: Option<Point<Pixels>> = None;
        let mut start = first_line;
        loop {
            let end = text[start..].find('\n').map_or(text.len(), |i| start + i);
            // Only lines that are laid out, i.e. on screen, have bounds.
            let line = state
                .range_to_bounds(&(start..start))
                .zip(state.range_to_bounds(&(end..end)));
            // A soft-wrapped line reaches the right edge of the text.
            let line = line.map(|(first, last)| match state.text_bounds() {
                Some(text) if last.top() > first.top() => point(text.right(), first.top()),
                _ => point(last.left(), first.top()),
            });
            match (line, beside) {
                (Some(line), Some(at)) => beside = Some(point(at.x.max(line.x), at.y.min(line.y))),
                (Some(line), None) => beside = Some(line),
                // Past the bottom of the screen: nothing further is shown.
                (None, Some(_)) => break,
                (None, None) => {}
            }
            if end >= last || end >= text.len() {
                break;
            }
            start = end + 1;
        }
        beside
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

    fn select(&self, range: Range<usize>, cx: &mut App) {
        self.state
            .update(cx, |state, cx| state.set_selected_range(range, cx));
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
