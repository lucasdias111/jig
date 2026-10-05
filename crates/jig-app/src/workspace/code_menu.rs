//! Right-clicking the code: the menu VS Code users reach for. Go to
//! Definition, Find All References and Rename Symbol for the name under the
//! pointer, a jig, the clipboard, and commenting lines out.

use std::ops::Range;

use gpui_kit::component::Icon;
use gpui_kit::component::input::{Copy, Cut, GoToDefinition, Paste, SelectAll};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::*;
use jig_editor::EditorHandle;

use super::commands::COMMAND_ICON;
use super::{FindReferences, OpenCommand, RenameSymbol, ToggleLineComment, Workspace};
use crate::definitions;
use crate::languages;

impl Workspace {
    /// Builds the menu for the current tab's editor. What it offers is
    /// settled when the frame is drawn: the editor is busy opening the menu
    /// when it's built, so it can't be asked then.
    pub(super) fn code_menu(
        &self,
        cx: &App,
    ) -> impl Fn(NativeMenu, &mut Window, &mut App) -> NativeMenu + 'static {
        let editable = !self.previewing();
        let language = self.document().language(&self.settings.languages);
        let commentable = editable && languages::line_comment(language).is_some();
        // The menu's actions go to whatever has focus; a right-click
        // doesn't always move it here.
        let focus = self.editor().state().focus_handle(cx);
        move |menu, window, cx| {
            window.focus(&focus, cx);
            menu.menu("Go to Definition", Box::new(GoToDefinition))
                .menu("Find All References", Box::new(FindReferences))
                .menu_with_disabled("Rename Symbol…", !editable, Box::new(RenameSymbol))
                .separator()
                // The sparkle the title bar's and the selection's Jig buttons wear.
                .menu_with_icon_disabled(
                    "Run Jig…",
                    Icon::default().data(COMMAND_ICON),
                    !editable,
                    Box::new(OpenCommand),
                )
                .separator()
                .menu_with_disabled("Cut", !editable, Box::new(Cut))
                .menu("Copy", Box::new(Copy))
                .menu_with_disabled("Paste", !editable, Box::new(Paste))
                .separator()
                .menu_with_disabled(
                    "Toggle Line Comment",
                    !commentable,
                    Box::new(ToggleLineComment),
                )
                .menu("Select All", Box::new(SelectAll))
        }
    }

    pub(super) fn find_references(
        &mut self,
        _: &FindReferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home {
            return;
        }
        let text = self.editor().text(cx);
        let Some(range) = definitions::name_at(&text, self.editor().cursor(cx)) else {
            self.show_note(
                "Put the cursor on a name to find its references.".into(),
                window,
                cx,
            );
            return;
        };
        if self.project_root(cx).is_none() {
            self.show_note(
                "Open a folder to find references across its files.".into(),
                window,
                cx,
            );
            return;
        }
        self.show_usages(&text[range.clone()], range.start, window, cx);
    }

    /// Comment out the selected lines (or the cursor's), or if they all are
    /// already, uncomment them. One undo step.
    pub(super) fn toggle_line_comment(
        &mut self,
        _: &ToggleLineComment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home || self.previewing() {
            return;
        }
        let language = self.document().language(&self.settings.languages);
        let Some(token) = languages::line_comment(language) else {
            return;
        };
        let text = self.editor().text(cx);
        let selection = self.editor().selection(cx);
        let edits = comment_edits(&text, selection.clone(), token);
        let (Some(first), Some(last)) = (edits.first(), edits.last()) else {
            return;
        };
        let span = first.0.start..last.0.end;
        let mut new = String::new();
        let mut at = span.start;
        for (range, insert) in &edits {
            new.push_str(&text[at..range.start]);
            new.push_str(insert);
            at = range.end;
        }
        self.editor().apply_edit(span, &new, window, cx);
        let start = shift(selection.start, &edits);
        let end = shift(selection.end, &edits);
        self.editor().select(start..end, cx);
    }
}

/// The edits that toggle `token` comments on the lines `selection` touches,
/// in order. Blank lines are left alone. A selection ending at the start
/// of a line leaves that line out, as when whole lines are selected.
fn comment_edits(text: &str, selection: Range<usize>, token: &str) -> Vec<(Range<usize>, String)> {
    let mut end = selection.end.min(text.len());
    if end > selection.start && text[..end].ends_with('\n') {
        end -= 1;
    }
    let mut lines = Vec::new();
    let mut start = text[..selection.start.min(end)]
        .rfind('\n')
        .map_or(0, |ix| ix + 1);
    loop {
        let line_end = text[start..].find('\n').map_or(text.len(), |ix| start + ix);
        let line = &text[start..line_end];
        if !line.trim().is_empty() {
            let indent = line.len() - line.trim_start().len();
            lines.push((start, indent, line));
        }
        if line_end >= end || line_end == text.len() {
            break;
        }
        start = line_end + 1;
    }
    let commented = !lines.is_empty()
        && lines
            .iter()
            .all(|(_, indent, line)| line[*indent..].starts_with(token));
    if commented {
        return lines
            .iter()
            .map(|(start, indent, line)| {
                let at = start + indent;
                let after = &line[indent + token.len()..];
                let len = token.len() + usize::from(after.starts_with(' '));
                (at..at + len, String::new())
            })
            .collect();
    }
    let indent = lines
        .iter()
        .map(|(_, indent, _)| *indent)
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|(start, _, _)| (start + indent..start + indent, format!("{token} ")))
        .collect()
}

/// Where `offset` ends up after `edits`, which are in order.
fn shift(offset: usize, edits: &[(Range<usize>, String)]) -> usize {
    let mut delta = 0isize;
    for (range, new) in edits {
        if range.end <= offset {
            delta += new.len() as isize - range.len() as isize;
        } else if range.start < offset {
            // In a removed comment marker: to where it was.
            return (range.start as isize + delta) as usize;
        }
    }
    (offset as isize + delta) as usize
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use super::{comment_edits, shift};

    fn toggle(text: &str, selection: Range<usize>) -> String {
        let mut out = text.to_string();
        for (range, insert) in comment_edits(text, selection, "//").into_iter().rev() {
            out.replace_range(range, &insert);
        }
        out
    }

    #[test]
    fn comments_at_the_shallowest_indent_and_skips_blank_lines() {
        let text = "fn a() {\n    if x {\n        y();\n\n    }\n}\n";
        let start = text.find("    if").unwrap();
        let end = text.find("}\n}").unwrap();
        assert_eq!(
            toggle(text, start..end),
            "fn a() {\n    // if x {\n    //     y();\n\n    // }\n}\n"
        );
    }

    #[test]
    fn uncomments_when_every_line_is_commented() {
        let text = "  // a\n  //b\n";
        assert_eq!(toggle(text, 0..text.len()), "  a\n  b\n");
        // One line without a comment: comment them all.
        let text = "// a\nb\n";
        assert_eq!(toggle(text, 0..text.len()), "// // a\n// b\n");
    }

    #[test]
    fn the_cursor_line_alone_without_a_selection() {
        let text = "a\nb\nc";
        assert_eq!(toggle(text, 2..2), "a\n// b\nc");
        assert_eq!(toggle(text, 5..5), "a\nb\n// c");
    }

    #[test]
    fn whole_lines_selected_leave_the_next_one_out() {
        let text = "a\nb\nc\n";
        assert_eq!(toggle(text, 0..4), "// a\n// b\nc\n");
    }

    #[test]
    fn cursor_moves_with_the_marker() {
        let text = "    x\n";
        let edits = comment_edits(text, 5..5, "//");
        assert_eq!(shift(5, &edits), 8);
        let text = "    // x\n";
        let edits = comment_edits(text, 8..8, "//");
        assert_eq!(shift(8, &edits), 5);
    }
}
