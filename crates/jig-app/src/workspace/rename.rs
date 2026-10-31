//! Rename Symbol (F2): the name under the cursor, renamed wherever the
//! file's language server says it's used, or without a server, everywhere
//! in this file. The edits land in open buffers, unsaved, one undo step per
//! file, so nothing reaches the disk before you've looked.

use std::ops::Range;
use std::path::PathBuf;
use std::time::Duration;

use futures::future::{Either, select};
use gpui_kit::*;
use jig_editor::EditorHandle;
use serde_json::{Value, json};

use super::{RenameSymbol, Workspace};
use jig_editing::text_edits::{Edits, shift, splice};
use jig_kit::field_box::{FieldBox, FieldBoxEvent};
use jig_lsp::definitions;
use jig_lsp::lsp::{self, Encoding};

/// How long to wait for the language server before renaming in this file.
const RENAME_TIMEOUT: Duration = Duration::from_secs(10);

/// Rename's field, while it's open. Its own entity, so the workspace only
/// opens it, places it and renames when it says so.
#[derive(Default)]
pub(super) struct RenameField {
    open: Option<OpenRename>,
}

struct OpenRename {
    view: Entity<FieldBox>,
    anchor: Point<Pixels>,
    /// The tab the name is in.
    tab: EntityId,
    /// Where the name was when the field opened.
    range: Range<usize>,
    _events: Subscription,
}

/// What the field asks of the workspace, which has the code and the focus.
pub(super) enum RenameEvent {
    /// Rename the name at `range` in tab `tab` to `name`. The field has
    /// closed.
    Rename {
        name: String,
        tab: EntityId,
        range: Range<usize>,
    },
    /// The field closed without renaming. `refocus` when it was dismissed,
    /// rather than left by clicking elsewhere.
    Closed { refocus: bool },
}

impl EventEmitter<RenameEvent> for RenameField {}

impl RenameField {
    pub(super) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Open the field at `anchor` on `name`, the one at `range` in tab
    /// `tab`, with the keyboard.
    fn open(
        &mut self,
        name: &str,
        tab: EntityId,
        range: Range<usize>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.new(|cx| FieldBox::new(name, "", "↩ rename · esc cancel", window, cx));
        let events = cx.subscribe(&view, |this, _, event, cx| match event {
            FieldBoxEvent::Submit(name) => {
                if let Some(open) = this.open.take() {
                    cx.emit(RenameEvent::Rename {
                        name: name.clone(),
                        tab: open.tab,
                        range: open.range,
                    });
                    cx.notify();
                }
            }
            FieldBoxEvent::Dismissed => this.close(true, cx),
            FieldBoxEvent::Blurred => this.close(false, cx),
        });
        self.open = Some(OpenRename {
            view,
            anchor,
            tab,
            range,
            _events: events,
        });
        cx.notify();
    }

    fn close(&mut self, refocus: bool, cx: &mut Context<Self>) {
        if self.open.take().is_some() {
            cx.emit(RenameEvent::Closed { refocus });
            cx.notify();
        }
    }
}

impl Render for RenameField {
    /// The field, under the name.
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let Some(open) = &self.open else {
            return div().into_any_element();
        };
        deferred(
            anchored()
                .position(open.anchor)
                .snap_to_window_with_margin(px(8.))
                .child(open.view.clone()),
        )
        .into_any_element()
    }
}

impl Workspace {
    pub(super) fn rename_symbol(
        &mut self,
        _: &RenameSymbol,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home || self.previewing() || self.modal_open() || self.rename.read(cx).is_open() {
            return;
        }
        let text = self.editor().text(cx);
        let Some(range) = definitions::name_at(&text, self.editor().cursor(cx)) else {
            self.show_note("Put the cursor on a name to rename it.".into(), window, cx);
            return;
        };
        let tab = self.tab().id();
        let anchor = self.floating_anchor(cx);
        self.rename.update(cx, |field, cx| {
            field.open(&text[range.clone()], tab, range, anchor, window, cx)
        });
        cx.notify();
    }

    /// Act on what the field says: rename, or back to the code.
    pub(super) fn on_rename_event(
        &mut self,
        _: &Entity<RenameField>,
        event: &RenameEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            RenameEvent::Rename { name, tab, range } => {
                cx.notify();
                self.finish_rename(name.clone(), *tab, range.clone(), window, cx);
            }
            RenameEvent::Closed { refocus } => {
                if *refocus {
                    self.focus_main(window, cx);
                }
                cx.notify();
            }
        }
    }

    fn finish_rename(
        &mut self,
        name: String,
        tab: EntityId,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home || self.tab().id() != tab {
            return;
        }
        self.focus_editor(window, cx);
        let text = self.editor().text(cx);
        let Some(lsp) = self.tab().lsp.as_ref() else {
            self.rename_in_file(range, &name, window, cx);
            return;
        };
        let client = lsp.client.clone();
        let encoding = client.encoding();
        let mut params =
            lsp::text_document_position(&lsp.uri, lsp::position(&text, range.start, encoding));
        params["newName"] = json!(name);
        let request = client.request("textDocument/rename", params);
        cx.spawn_in(window, async move |this, cx| {
            let timer = cx.background_executor().timer(RENAME_TIMEOUT);
            let response = match select(request, Box::pin(timer)).await {
                Either::Left((Ok(response), _)) => Some(response),
                _ => None,
            };
            this.update_in(cx, |this, window, cx| {
                // Typed into since: the server's answer is about other text.
                if this.home || this.tab().id() != tab || this.editor().text(cx) != text {
                    return;
                }
                match response {
                    Some(Err(error)) => {
                        this.show_note(format!("Can't rename: {error}"), window, cx)
                    }
                    Some(Ok(edit)) if !workspace_edits(&edit).is_empty() => {
                        this.apply_rename(&edit, encoding, range.start, window, cx)
                    }
                    // No answer, or none it could give: this file's uses.
                    _ => this.rename_in_file(range, &name, window, cx),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Rename every whole-word use of the name at `range` in this file.
    fn rename_in_file(
        &mut self,
        range: Range<usize>,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = self.editor().text(cx);
        let edits = word_edits(&text, &text[range.clone()], name);
        let uses = edits.len();
        let cursor = self.editor().cursor(cx);
        if let Some(cursor) = apply(self.editor(), &text, edits, Some(cursor), window, cx) {
            self.editor().select(cursor..cursor, cx);
        }
        let note = if self.tab().lsp.is_some() {
            format!("Renamed {} in this file.", count(uses, "use"))
        } else {
            format!(
                "Renamed {} in this file; no language server for the others.",
                count(uses, "use")
            )
        };
        self.show_note(note, window, cx);
    }

    fn apply_rename(
        &mut self,
        edit: &Value,
        encoding: Encoding,
        cursor: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (files, uses) = self.apply_workspace_edit(edit, encoding, cursor, window, cx);
        let note = if files > 1 {
            format!(
                "Renamed {} in {files} files. Save to keep them.",
                count(uses, "use")
            )
        } else {
            format!("Renamed {}.", count(uses, "use"))
        };
        self.show_note(note, window, cx);
    }

    /// Make a server's `WorkspaceEdit` in each file's buffer, one undo step
    /// per file, opening tabs, without showing them, for files that aren't
    /// open. `cursor` in the current tab follows the edits. Returns how
    /// many files and edits it changed.
    pub(super) fn apply_workspace_edit(
        &mut self,
        edit: &Value,
        encoding: Encoding,
        cursor: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (usize, usize) {
        let current = self.tab().id();
        let mut files = 0;
        let mut changed = 0;
        for (path, edits) in workspace_edits(edit) {
            let ix = match self.tab_for(&path) {
                Some(ix) => ix,
                None => match self.open_in_background(&path, window, cx) {
                    Some(ix) => ix,
                    None => continue,
                },
            };
            let editor = self.tabs[ix].editor.clone();
            let text = editor.text(cx);
            let edits: Edits = edits
                .into_iter()
                .map(|(range, new)| (lsp::range(&text, range, encoding), new))
                .collect();
            let count = edits.len();
            let here = self.tabs[ix].id() == current;
            let moved = apply(&editor, &text, edits, here.then_some(cursor), window, cx);
            if let Some(cursor) = moved.filter(|_| here) {
                editor.select(cursor..cursor, cx);
            }
            if moved.is_some() {
                files += 1;
                changed += count;
            }
        }
        self.focus_editor(window, cx);
        (files, changed)
    }

    /// The field, under the name, while it's open.
    pub(super) fn render_rename(&self, cx: &App) -> Option<AnyElement> {
        self.rename
            .read(cx)
            .is_open()
            .then(|| self.rename.clone().into_any_element())
    }
}

/// Make `edits` to `editor`'s `text` as one undo step. Returns where
/// `cursor` ends up, or `None` if there was nothing to do.
fn apply(
    editor: &jig_editor::KitEditor,
    text: &str,
    mut edits: Edits,
    cursor: Option<usize>,
    window: &mut Window,
    cx: &mut App,
) -> Option<usize> {
    edits.sort_by_key(|(range, _)| range.start);
    let (span, new) = splice(text, &edits)?;
    editor.apply_edit(span, &new, window, cx);
    Some(cursor.map_or(0, |cursor| shift(cursor, &edits)))
}

/// Every whole-word `old` in `text`, to become `new`.
fn word_edits(text: &str, old: &str, new: &str) -> Edits {
    super::occurrences::whole_words(text, old)
        .into_iter()
        .map(|range| (range, new.to_string()))
        .collect()
}

/// A `WorkspaceEdit`'s text edits by file, in the server's positions. File
/// creations, renames and deletions are left out.
pub(super) fn workspace_edits(edit: &Value) -> Vec<(PathBuf, Vec<(lsp_types::Range, String)>)> {
    let text_edits = |edits: &Value| -> Vec<(lsp_types::Range, String)> {
        edits
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|edit| {
                let range = serde_json::from_value(edit.get("range")?.clone()).ok()?;
                Some((range, edit.get("newText")?.as_str()?.to_string()))
            })
            .collect()
    };
    let files: Vec<(PathBuf, Vec<_>)> = match edit.get("documentChanges") {
        Some(changes) => changes
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|change| {
                let uri = change.get("textDocument")?.get("uri")?.as_str()?;
                Some((lsp::uri_path(uri)?, text_edits(change.get("edits")?)))
            })
            .collect(),
        None => edit
            .get("changes")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(uri, edits)| Some((lsp::uri_path(uri)?, text_edits(edits))))
            .collect(),
    };
    files
        .into_iter()
        .filter(|(_, edits)| !edits.is_empty())
        .collect()
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use super::{word_edits, workspace_edits};
    use jig_editing::text_edits::{Edits, shift, splice};

    fn rename(text: &str, edits: &Edits) -> String {
        let (span, new) = splice(text, edits).unwrap();
        format!("{}{new}{}", &text[..span.start], &text[span.end..])
    }

    #[test]
    fn whole_words_are_renamed() {
        let text = "let total = 1;\nlet subtotal = total + total2;\nuse(total);\n";
        let edits = word_edits(text, "total", "sum");
        assert_eq!(
            rename(text, &edits),
            "let sum = 1;\nlet subtotal = sum + total2;\nuse(sum);\n"
        );
    }

    #[test]
    fn cursor_follows_the_edits() {
        let text = "a(total, total)";
        let edits = word_edits(text, "total", "n");
        // Three letters into the second use: the end of its `n`.
        assert_eq!(shift(12, &edits), 6);
        // Past both.
        assert_eq!(shift(text.len(), &edits), "a(n, n)".len());
        // Before them.
        assert_eq!(shift(1, &edits), 1);
    }

    #[test]
    fn workspace_edits_in_both_shapes() {
        let range =
            json!({"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 4}});
        let changes = json!({"changes": {"file:///p/a.rs": [{"range": range, "newText": "b"}]}});
        let documents = json!({"documentChanges": [
            {"textDocument": {"uri": "file:///p/a.rs", "version": 1},
             "edits": [{"range": range, "newText": "b"}]},
            {"kind": "rename", "oldUri": "file:///p/x.rs", "newUri": "file:///p/y.rs"},
        ]});
        for edit in [changes, documents] {
            let files = workspace_edits(&edit);
            assert_eq!(files.len(), 1);
            assert_eq!(files[0].0, PathBuf::from("/p/a.rs"));
            assert_eq!(files[0].1[0].1, "b");
        }
        assert!(workspace_edits(&Value::Null).is_empty());
    }
}
