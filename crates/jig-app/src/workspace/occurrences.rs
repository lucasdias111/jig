//! The other uses of the name at the cursor, or of the name selected, get a
//! soft fill: where the file's language server says they are
//! (`textDocument/documentHighlight`), or without one, every whole-word
//! match in the file.

use std::ops::Range;
use std::time::Duration;

use gpui_kit::component::input::{
    EditorState, RangeDecoration, RangeDecorationCollection, RangeDecorationStyle,
};
use gpui_kit::*;
use regex::Regex;
use serde_json::Value;

use super::Workspace;
use super::definitions::with_timeout;
use crate::definitions;
use crate::lsp::{self, Encoding};

/// How long the cursor rests on a name before its uses are looked for, so
/// moving through the code doesn't flicker.
const DELAY: Duration = Duration::from_millis(150);
/// Past this, the words in the file stand in for the server's answer.
const TIMEOUT: Duration = Duration::from_secs(1);
/// Files bigger than this aren't searched.
const MAX_BYTES: usize = 4 * 1024 * 1024;

/// A tab's highlighted uses, and the name they're for.
pub(super) struct Occurrences {
    marks: RangeDecorationCollection,
    name: Option<Range<usize>>,
    task: Task<()>,
}

impl Occurrences {
    pub(super) fn new(state: &Entity<EditorState>, cx: &mut App) -> Self {
        let marks = state.update(cx, |state, cx| {
            state.create_range_decorations_collection(Vec::new(), cx)
        });
        Self {
            marks,
            name: None,
            task: Task::ready(()),
        }
    }

    fn clear(&mut self, cx: &mut App) {
        self.name = None;
        self.task = Task::ready(());
        self.marks.set(Vec::new(), cx);
    }
}

impl super::tabs::Tab {
    /// Where the highlighted uses are, for tests.
    #[cfg(test)]
    pub(super) fn occurrences(&self, cx: &App) -> Vec<Range<usize>> {
        self.occurrences.marks.get_ranges(cx)
    }
}

impl Workspace {
    /// Highlight the uses of the name at tab `ix`'s cursor, after a moment;
    /// `edited` when the text changed, which leaves the old ones wrong.
    pub(super) fn show_occurrences(&mut self, ix: usize, edited: bool, cx: &mut Context<Self>) {
        let reviewing = ix == self.active && self.previewing();
        let tab = &mut self.tabs[ix];
        if edited {
            tab.occurrences.clear(cx);
        }
        let editor = tab.editor.state().read(cx);
        let length = editor.text().len();
        let selection = editor.selected_range();
        let cursor = editor.cursor();
        let text = if length <= MAX_BYTES && !reviewing {
            editor.text().to_string()
        } else {
            String::new()
        };
        let Some(name) = target(&text, selection, cursor) else {
            tab.occurrences.clear(cx);
            return;
        };
        if tab.occurrences.name.as_ref() == Some(&name) {
            return;
        }
        // Onto another use of the same name: what's shown still holds.
        if tab.occurrences.marks.get_ranges(cx).contains(&name) {
            tab.occurrences.name = Some(name);
            return;
        }
        tab.occurrences.clear(cx);
        tab.occurrences.name = Some(name.clone());

        let id = tab.id();
        let server = tab.lsp.as_ref().map(|lsp| {
            let encoding = lsp.client.encoding();
            let params =
                lsp::text_document_position(&lsp.uri, lsp::position(&text, name.start, encoding));
            (lsp.client.clone(), params, encoding)
        });
        tab.occurrences.task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DELAY).await;
            let mut found = None;
            if let Some((client, params, encoding)) = server {
                let request = client.request("textDocument/documentHighlight", params);
                found = with_timeout(request, TIMEOUT, cx)
                    .await
                    .and_then(|answer| highlights(&text, &answer, encoding));
            }
            let found = found.unwrap_or_else(|| whole_words(&text, &text[name.clone()]));
            this.update(cx, |this, cx| {
                let Some(tab) = this.tabs.iter_mut().find(|tab| tab.id() == id) else {
                    return;
                };
                if tab.occurrences.name.as_ref() != Some(&name) {
                    return;
                }
                // Just the name itself is nothing to point out.
                let marks = if found.len() > 1 {
                    found
                        .into_iter()
                        .map(|range| {
                            RangeDecoration::new(range).with_style(RangeDecorationStyle::Fill)
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                tab.occurrences.marks.set(marks, cx);
            })
            .ok();
        });
    }
}

/// The name whose uses to show: the one at the cursor, or the selection
/// when it's exactly a name.
fn target(text: &str, selection: Range<usize>, cursor: usize) -> Option<Range<usize>> {
    if selection.is_empty() {
        return definitions::name_at(text, cursor);
    }
    definitions::name_at(text, selection.start).filter(|name| *name == selection)
}

/// Every whole-word `name` in `text`.
pub(super) fn whole_words(text: &str, name: &str) -> Vec<Range<usize>> {
    let Ok(pattern) = Regex::new(&format!(r"\b{}\b", regex::escape(name))) else {
        return Vec::new();
    };
    pattern.find_iter(text).map(|m| m.range()).collect()
}

/// The ranges in a `documentHighlight` answer; `None` when the server had
/// none to give.
fn highlights(text: &str, answer: &Value, encoding: Encoding) -> Option<Vec<Range<usize>>> {
    let ranges = answer
        .as_array()?
        .iter()
        .filter_map(|highlight| serde_json::from_value(highlight.get("range")?.clone()).ok())
        .map(|range| lsp::range(text, range, encoding))
        .filter(|range| !range.is_empty())
        .collect();
    Some(ranges)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{highlights, target, whole_words};
    use crate::lsp::Encoding;

    #[test]
    fn the_target_is_the_name_at_the_cursor_or_selected() {
        let text = "let total = sub(total);";
        assert_eq!(target(text, 6..6, 6), Some(4..9));
        assert_eq!(target(text, 4..9, 9), Some(4..9));
        // Part of a name, or more than one, isn't a name.
        assert_eq!(target(text, 4..7, 7), None);
        assert_eq!(target(text, 4..15, 15), None);
        // Keywords aren't names.
        assert_eq!(target(text, 1..1, 1), None);
    }

    #[test]
    fn whole_words_only() {
        let text = "total + subtotal + total2 + total";
        assert_eq!(whole_words(text, "total"), vec![0..5, 28..33]);
    }

    #[test]
    fn highlights_from_the_server() {
        let text = "a b a\n";
        let range = |start, end| {
            json!({"range": {"start": {"line": 0, "character": start},
                             "end": {"line": 0, "character": end}}, "kind": 1})
        };
        let answer = json!([range(0, 1), range(4, 5)]);
        assert_eq!(
            highlights(text, &answer, Encoding::Utf16),
            Some(vec![0..1, 4..5])
        );
        assert_eq!(highlights(text, &json!(null), Encoding::Utf16), None);
    }
}
