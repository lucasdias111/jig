//! The strip along the bottom of the editor: where the cursor is, and how
//! the file is indented, encoded and highlighted.

use gpui_kit::component::input::EditorState;
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::*;

use super::Workspace;
use crate::{indentation, languages};

pub(super) const HEIGHT: f32 = 24.;

/// `Ln 12, Col 5`, for one tab's editor. A view of its own so the cursor
/// moving repaints just this, not the whole workspace.
pub(super) struct CursorPosition {
    editor: Entity<EditorState>,
    _observe: Subscription,
}

impl CursorPosition {
    pub(super) fn new(editor: Entity<EditorState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&editor, |_, _, cx| cx.notify());
        Self {
            editor,
            _observe: observe,
        }
    }
}

impl Render for CursorPosition {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.editor.read(cx);
        let position = state.cursor_position();
        let selection = state.selected_range();
        let mut label = format!("Ln {}, Col {}", position.line + 1, position.character + 1);
        if !selection.is_empty() {
            let text = state.text();
            let selected =
                text.byte_to_char_idx(selection.end) - text.byte_to_char_idx(selection.start);
            label.push_str(&format!(" ({selected} selected)"));
        }
        div().child(label)
    }
}

impl Workspace {
    pub(super) fn render_status_bar(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let tab = self.tab();
        let language = tab
            .document
            .path
            .as_deref()
            .and_then(|path| languages::written_in(path, &self.settings.languages))
            .map_or("Plain Text", |language| language.label);
        let item = |text: SharedString| div().child(text);
        h_flex()
            .flex_none()
            .h(px(HEIGHT))
            .px_3()
            .gap_4()
            .justify_end()
            .border_t_1()
            .border_color(theme.title_bar_border)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(tab.cursor.clone())
            .child(item(indentation::label(tab.indentation).into()))
            .child(item(tab.document.encoding().into()))
            .child(item(tab.document.line_ending().into()))
            .child(item(language.into()))
    }
}
