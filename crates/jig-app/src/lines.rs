//! Whole-line edits: move, duplicate, delete, select and open lines, and
//! finding a line by number. Plain text in, one edit out, so the editor
//! makes each an undo step of its own.

use std::ops::Range;

/// Replace `range` with `text`, then select `selection` (in the new text).
#[derive(Debug, PartialEq, Eq)]
pub struct LineEdit {
    pub range: Range<usize>,
    pub text: String,
    pub selection: Range<usize>,
}

/// The lines `selection` touches, from the start of the first to the end
/// of the last, without its newline. A selection ending at the start of a
/// line leaves that line out, as when whole lines are selected.
pub fn line_span(text: &str, selection: Range<usize>) -> Range<usize> {
    let mut end = selection.end.min(text.len());
    let start = selection.start.min(end);
    if end > start && text[..end].ends_with('\n') {
        end -= 1;
    }
    let first = line_start(text, start);
    let last = text[end..].find('\n').map_or(text.len(), |ix| end + ix);
    first..last
}

fn line_start(text: &str, offset: usize) -> usize {
    text[..offset].rfind('\n').map_or(0, |ix| ix + 1)
}

fn shifted(selection: &Range<usize>, by: isize) -> Range<usize> {
    let move_by = |offset: usize| (offset as isize + by) as usize;
    move_by(selection.start)..move_by(selection.end)
}

/// Swap the lines `selection` touches with the one above or below; `None`
/// at the top or bottom of the file.
pub fn move_lines(text: &str, selection: Range<usize>, up: bool) -> Option<LineEdit> {
    let span = line_span(text, selection.clone());
    let lines = &text[span.clone()];
    if up {
        if span.start == 0 {
            return None;
        }
        let above = line_start(text, span.start - 1);
        let text = format!("{lines}\n{}", &text[above..span.start - 1]);
        Some(LineEdit {
            range: above..span.end,
            text,
            selection: shifted(&selection, -((span.start - above) as isize)),
        })
    } else {
        if span.end == text.len() {
            return None;
        }
        let below = span.end + 1;
        let below_end = text[below..].find('\n').map_or(text.len(), |ix| below + ix);
        let text = format!("{}\n{lines}", &text[below..below_end]);
        Some(LineEdit {
            range: span.start..below_end,
            text,
            selection: shifted(&selection, (below_end - span.end) as isize),
        })
    }
}

/// Copy the lines `selection` touches below themselves, selecting the
/// same in the copy.
pub fn duplicate_lines(text: &str, selection: Range<usize>) -> LineEdit {
    let span = line_span(text, selection.clone());
    let copy = format!("\n{}", &text[span.clone()]);
    LineEdit {
        range: span.end..span.end,
        selection: shifted(&selection, copy.len() as isize),
        text: copy,
    }
}

/// Remove the lines `selection` touches, leaving the cursor at the same
/// column of the line that takes their place.
pub fn delete_lines(text: &str, selection: Range<usize>) -> LineEdit {
    let span = line_span(text, selection.clone());
    let column = selection.start.min(text.len()) - span.start;
    let range = if span.end < text.len() {
        span.start..span.end + 1
    } else if span.start > 0 {
        span.start - 1..span.end
    } else {
        span.clone()
    };
    let mut after = text.to_string();
    after.replace_range(range.clone(), "");
    let start = line_start(&after, range.start.min(after.len()));
    let end = after[start..]
        .find('\n')
        .map_or(after.len(), |ix| start + ix);
    let mut cursor = (start + column).min(end);
    while !after.is_char_boundary(cursor) {
        cursor -= 1;
    }
    LineEdit {
        range,
        text: String::new(),
        selection: cursor..cursor,
    }
}

/// Select the cursor's line up to its end, so the cursor stays on it;
/// with whole lines selected already, take in the next one too.
pub fn select_lines(text: &str, selection: Range<usize>) -> Range<usize> {
    let span = line_span(text, selection.clone());
    // A blank line selected is an empty selection, so it counts as whole.
    let whole = selection == span && (!selection.is_empty() || span.is_empty());
    if whole && span.end < text.len() {
        let next = span.end + 1;
        let next_end = text[next..].find('\n').map_or(text.len(), |ix| next + ix);
        return span.start..next_end;
    }
    span
}

/// A new line below (or above) the lines `selection` touches, indented as
/// the line beside it, with the cursor on it.
pub fn open_line(text: &str, selection: Range<usize>, below: bool) -> LineEdit {
    let span = line_span(text, selection);
    let beside = if below {
        line_start(text, span.end)
    } else {
        span.start
    };
    let line = &text[beside..];
    let line = &line[..line.find('\n').unwrap_or(line.len())];
    let indent = &line[..line.len() - line.trim_start().len()];
    if below {
        let cursor = span.end + 1 + indent.len();
        LineEdit {
            range: span.end..span.end,
            text: format!("\n{indent}"),
            selection: cursor..cursor,
        }
    } else {
        let cursor = span.start + indent.len();
        LineEdit {
            range: span.start..span.start,
            text: format!("{indent}\n"),
            selection: cursor..cursor,
        }
    }
}

/// Where `target` points: `42` or `42:5`, both counted from 1. Past the
/// end goes to the last line, and a column past the line to its end.
pub fn offset_of(text: &str, target: &str) -> Option<usize> {
    let (line, column) = match target.trim().split_once([':', ',']) {
        Some((line, column)) => (line, Some(column)),
        None => (target.trim(), None),
    };
    let line: usize = line.trim().parse().ok()?;
    let column: usize = match column {
        Some(column) => column.trim().parse().ok()?,
        None => 1,
    };
    let mut start = 0;
    for _ in 1..line.max(1) {
        match text[start..].find('\n') {
            Some(ix) => start += ix + 1,
            None => break,
        }
    }
    let line = &text[start..];
    let line = &line[..line.find('\n').unwrap_or(line.len())];
    let within = line
        .char_indices()
        .nth(column.max(1) - 1)
        .map_or(line.len(), |(ix, _)| ix);
    Some(start + within)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(text: &str, edit: &LineEdit) -> String {
        let mut out = text.to_string();
        out.replace_range(edit.range.clone(), &edit.text);
        out
    }

    #[test]
    fn moves_the_cursor_line_and_the_cursor_with_it() {
        let text = "a\nbb\nc";
        let edit = move_lines(text, 3..3, true).unwrap();
        assert_eq!(apply(text, &edit), "bb\na\nc");
        assert_eq!(edit.selection, 1..1);
        let edit = move_lines(text, 3..3, false).unwrap();
        assert_eq!(apply(text, &edit), "a\nc\nbb");
        assert_eq!(edit.selection, 5..5);
    }

    #[test]
    fn moves_every_selected_line_but_not_past_the_ends() {
        let text = "a\nb\nc\n";
        // Whole lines a and b, ending at the start of c.
        let edit = move_lines(text, 0..4, false).unwrap();
        assert_eq!(apply(text, &edit), "c\na\nb\n");
        assert_eq!(edit.selection, 2..6);
        assert_eq!(move_lines(text, 0..1, true), None);
        assert_eq!(move_lines("a\nb", 2..2, false), None);
    }

    #[test]
    fn duplicates_below_and_follows_the_copy() {
        let text = "  x\ny";
        let edit = duplicate_lines(text, 2..3);
        assert_eq!(apply(text, &edit), "  x\n  x\ny");
        assert_eq!(edit.selection, 6..7);
    }

    #[test]
    fn deletes_lines_keeping_the_column() {
        let text = "one\ntwo\nthree";
        let edit = delete_lines(text, 6..6);
        assert_eq!(apply(text, &edit), "one\nthree");
        assert_eq!(edit.selection, 6..6);
        // The last line takes the newline before it.
        let edit = delete_lines(text, 10..10);
        assert_eq!(apply(text, &edit), "one\ntwo");
        assert_eq!(edit.selection, 6..6);
        assert_eq!(apply("solo", &delete_lines("solo", 2..2)), "");
    }

    #[test]
    fn selecting_lines_grows_a_line_at_a_time() {
        let text = "a\nb\nc";
        assert_eq!(select_lines(text, 0..0), 0..1);
        assert_eq!(select_lines(text, 0..1), 0..3);
        assert_eq!(select_lines(text, 0..3), 0..5);
        assert_eq!(select_lines(text, 0..5), 0..5);
        assert_eq!(select_lines(text, 3..3), 2..3);
        // A blank line grows into the next one.
        assert_eq!(select_lines("a\n\nb", 2..2), 2..4);
    }

    #[test]
    fn opens_lines_at_the_indent_beside_them() {
        let text = "    x\ny";
        let edit = open_line(text, 2..2, true);
        assert_eq!(apply(text, &edit), "    x\n    \ny");
        assert_eq!(edit.selection, 10..10);
        let edit = open_line(text, 2..2, false);
        assert_eq!(apply(text, &edit), "    \n    x\ny");
        assert_eq!(edit.selection, 4..4);
    }

    #[test]
    fn finds_lines_and_columns_from_one() {
        let text = "ab\ncd\nef";
        assert_eq!(offset_of(text, "2"), Some(3));
        assert_eq!(offset_of(text, "2:2"), Some(4));
        assert_eq!(offset_of(text, " 3 : 9 "), Some(8));
        assert_eq!(offset_of(text, "99"), Some(6));
        assert_eq!(offset_of(text, "x"), None);
    }
}
