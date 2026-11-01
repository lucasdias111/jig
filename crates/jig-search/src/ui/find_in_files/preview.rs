//! The selected match's file, shown read-only below the list with the
//! match selected.

use super::*;

/// The selected match's file, shown read-only below the list.
pub(super) struct Preview {
    pub(super) path: String,
    pub(super) editor: KitEditor,
}

impl FindInFiles {
    pub(super) fn preview_focused(&self, window: &Window, cx: &App) -> bool {
        self.preview.as_ref().is_some_and(|preview| {
            preview
                .editor
                .state()
                .focus_handle(cx)
                .contains_focused(window, cx)
        })
    }

    /// Show the selected match in the preview, loading its file if it
    /// isn't the one already shown.
    pub(super) fn update_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(found) = self.results.matches.get(self.selected).cloned() else {
            self.preview = None;
            return;
        };
        if self.preview.as_ref().is_none_or(|p| p.path != found.path) {
            let full = self.root.join(&found.path);
            let Some(text) = persist::read(&self.root, &found.path) else {
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
}
