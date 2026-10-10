//! Where the cursor is in the code: the bracket next to it and the one it
//! pairs with get a box each, and the indent guide of the block it's in is
//! drawn stronger than the others.

use gpui_kit::base::input::ActiveIndentGuide;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::input::{
    EditorState, RangeDecoration, RangeDecorationCollection, RangeDecorationStyle,
};
use gpui_kit::*;

use super::Workspace;

/// Files bigger than this aren't scanned on every cursor move.
const MAX_BYTES: usize = 4 * 1024 * 1024;

/// A tab's bracket boxes, and what they were last worked out for.
pub(super) struct Structure {
    boxes: RangeDecorationCollection,
    /// The cursor and text length last shown for, so the editor repainting
    /// for anything else doesn't redo the work.
    seen: Option<(usize, usize)>,
    _observe: Subscription,
}

impl super::tabs::Tab {
    /// Where the bracket boxes are, for tests.
    #[cfg(test)]
    pub(super) fn structure_boxes(&self, cx: &App) -> Vec<std::ops::Range<usize>> {
        self.structure.boxes.get_ranges(cx)
    }
}

impl Workspace {
    pub(super) fn install_structure(
        &mut self,
        state: &Entity<EditorState>,
        cx: &mut Context<Self>,
    ) -> Structure {
        let boxes = state.update(cx, |state, cx| {
            state.create_range_decorations_collection(Vec::new(), cx)
        });
        let observe = cx.observe(state, |this, state, cx| {
            if let Some(ix) = this
                .tabs
                .iter()
                .position(|tab| tab.editor.state() == &state)
            {
                this.show_structure(ix, false, cx);
            }
        });
        Structure {
            boxes,
            seen: None,
            _observe: observe,
        }
    }

    /// Box the brackets at the cursor and mark its block's guide in tab
    /// `ix`; `again` after edits that may leave cursor and length the same.
    pub(super) fn show_structure(&mut self, ix: usize, again: bool, cx: &mut Context<Self>) {
        let tab = &self.tabs[ix];
        let state = tab.editor.state().clone();
        let editor = state.read(cx);
        let cursor = editor.cursor();
        let seen = (cursor, editor.text().len());
        if !again && tab.structure.seen == Some(seen) {
            return;
        }
        let text = if seen.1 <= MAX_BYTES {
            editor.text().to_string()
        } else {
            String::new()
        };
        let language = tab.document.language(&self.settings.languages);
        let comment = crate::languages::line_comment(language);
        let pair = crate::brackets::matching(&text, cursor, comment);
        let guide = self
            .settings
            .editor
            .indent_guides
            .then(|| {
                let row = text[..cursor.min(text.len())].matches('\n').count();
                crate::indentation::active_guide(&text, row, tab.indentation.tab_size)
            })
            .flatten();

        let theme = cx.theme();
        let frame = theme.muted_foreground.opacity(0.6);
        let guide_color = theme.muted_foreground.opacity(0.55);
        let boxes = pair
            .map(|(open, close)| [open..open + 1, close..close + 1])
            .into_iter()
            .flatten()
            .map(|range| {
                RangeDecoration::new(range)
                    .with_style(RangeDecorationStyle::Frame)
                    .with_color(frame)
            })
            .collect();
        let guide = guide.map(|(rows, column)| ActiveIndentGuide {
            rows,
            column,
            color: guide_color,
        });
        let tab = &mut self.tabs[ix];
        tab.structure.seen = Some(seen);
        tab.structure.boxes.set(boxes, cx);
        state.update(cx, |state, cx| state.set_active_indent_guide(guide, cx));
    }
}
