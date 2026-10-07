//! Edits to a buffer's text as byte ranges and what replaces them: made as
//! one undo step, with the cursor carried along. Rename and formatting
//! share them; formatting also turns a server's `TextEdit`s, or a
//! formatter's whole new text, into them.

use std::ops::Range;

use serde_json::Value;

use crate::lsp::{self, Encoding};

/// Byte ranges and what replaces them.
pub type Edits = Vec<(Range<usize>, String)>;

/// Sorted, non-overlapping `edits` to `text` as one: the span from the
/// first to the last, and what replaces it.
pub fn splice(text: &str, edits: &[(Range<usize>, String)]) -> Option<(Range<usize>, String)> {
    if edits.windows(2).any(|pair| pair[0].0.end > pair[1].0.start) {
        return None;
    }
    let span = edits.first()?.0.start..edits.last()?.0.end;
    let mut new = String::new();
    let mut at = span.start;
    for (range, replacement) in edits {
        new.push_str(text.get(at..range.start)?);
        new.push_str(replacement);
        at = range.end;
    }
    Some((span, new))
}

/// Where `offset` ends up after sorted `edits`: moved along by those before
/// it, and kept as far into the one it's in as the new text allows.
pub fn shift(offset: usize, edits: &[(Range<usize>, String)]) -> usize {
    let mut delta = 0isize;
    for (range, new) in edits {
        if range.end <= offset {
            delta += new.len() as isize - range.len() as isize;
        } else if range.start <= offset {
            let into = (offset - range.start).min(new.len());
            return (range.start as isize + delta) as usize + into;
        }
    }
    (offset as isize + delta) as usize
}

/// Where `offset` in `old` ends up after sorted `edits`, as [`shift`], but
/// inside an edit it keeps to the same code rather than the same column:
/// past as many characters that aren't whitespace. Formatting moves mostly
/// whitespace, so the cursor stays on the token it was on.
pub fn carry(old: &str, offset: usize, edits: &[(Range<usize>, String)]) -> usize {
    let Some((range, new)) = edits
        .iter()
        .find(|(range, _)| range.start < offset && offset < range.end)
    else {
        return shift(offset, edits);
    };
    let code = old[range.start..offset]
        .chars()
        .filter(|c| !c.is_whitespace())
        .count();
    let mut into = new.len();
    let mut seen = 0;
    for (ix, c) in new.char_indices() {
        if seen == code {
            into = ix;
            break;
        }
        if !c.is_whitespace() {
            seen += 1;
        }
    }
    // Before the edit's whitespace that precedes the next code, as the
    // cursor was.
    if old[offset..range.end].starts_with(|c: char| !c.is_whitespace()) {
        into += new[into..].len() - new[into..].trim_start().len();
    }
    shift(range.start, edits) + into
}

/// `text` with sorted `edits` made, or `None` if they overlap.
pub fn applied(text: &str, edits: &[(Range<usize>, String)]) -> Option<String> {
    if edits.is_empty() {
        return Some(text.to_string());
    }
    let (span, new) = splice(text, edits)?;
    Some(format!("{}{new}{}", &text[..span.start], &text[span.end..]))
}

/// A server's `TextEdit[]` as byte edits to `text`, sorted. They all refer
/// to `text` as it is, in any order; inserts at the same place keep theirs.
pub fn from_lsp(text: &str, edits: &Value, encoding: Encoding) -> Edits {
    let mut edits: Edits = edits
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|edit| {
            let range = serde_json::from_value(edit.get("range")?.clone()).ok()?;
            let new = edit.get("newText")?.as_str()?.to_string();
            Some((lsp::range(text, range, encoding), new))
        })
        .collect();
    edits.sort_by_key(|(range, _)| range.start);
    edits
}

/// Lines past these aren't matched one by one; what lies between the
/// common start and end becomes one edit.
const MAX_LINES: usize = 3000;

/// The edits that turn `old` into `new`, one per run of changed lines, so a
/// cursor on a line nothing touched stays on it.
pub fn line_edits(old: &str, new: &str) -> Edits {
    let a: Vec<&str> = old.split_inclusive('\n').collect();
    let b: Vec<&str> = new.split_inclusive('\n').collect();
    let prefix = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let suffix = a[prefix..]
        .iter()
        .rev()
        .zip(b[prefix..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let (a_mid, b_mid) = (&a[prefix..a.len() - suffix], &b[prefix..b.len() - suffix]);
    let start: usize = a[..prefix].iter().map(|line| line.len()).sum();
    if a_mid.is_empty() && b_mid.is_empty() {
        return Vec::new();
    }
    if a_mid.len() > MAX_LINES || b_mid.len() > MAX_LINES {
        let old_len: usize = a_mid.iter().map(|line| line.len()).sum();
        return vec![(start..start + old_len, b_mid.concat())];
    }
    // lcs[i][j]: the longest common run of a_mid[i..] and b_mid[j..].
    let mut lcs = vec![vec![0u32; b_mid.len() + 1]; a_mid.len() + 1];
    for i in (0..a_mid.len()).rev() {
        for j in (0..b_mid.len()).rev() {
            lcs[i][j] = if a_mid[i] == b_mid[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    let mut edits = Edits::new();
    let mut pending: Option<(Range<usize>, String)> = None;
    let (mut i, mut j, mut at) = (0, 0, start);
    while i < a_mid.len() || j < b_mid.len() {
        if i < a_mid.len() && j < b_mid.len() && a_mid[i] == b_mid[j] {
            edits.extend(pending.take());
            at += a_mid[i].len();
            i += 1;
            j += 1;
        } else {
            let edit = pending.get_or_insert_with(|| (at..at, String::new()));
            if j < b_mid.len() && (i == a_mid.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
                edit.1.push_str(b_mid[j]);
                j += 1;
            } else {
                at += a_mid[i].len();
                edit.0.end = at;
                i += 1;
            }
        }
    }
    edits.extend(pending);
    edits
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn at(line: u32, start: u32, end_line: u32, end: u32) -> Value {
        json!({"start": {"line": line, "character": start}, "end": {"line": end_line, "character": end}})
    }

    #[test]
    fn several_server_edits_in_reverse_order() {
        let text = "fn a(){\nlet x=1;\n}\n";
        // As servers often send them: last first.
        let edits = json!([
            {"range": at(1, 5, 1, 6), "newText": " = "},
            {"range": at(1, 0, 1, 0), "newText": "    "},
            {"range": at(0, 6, 0, 6), "newText": " "},
        ]);
        let edits = from_lsp(text, &edits, Encoding::Utf16);
        assert_eq!(edits[0].0, 6..6, "sorted");
        assert_eq!(
            applied(text, &edits).unwrap(),
            "fn a() {\n    let x = 1;\n}\n"
        );
    }

    #[test]
    fn positions_count_utf16_units() {
        // `é` is one UTF-16 unit and two bytes; `😀` two units, four bytes.
        let text = "let é = \"😀\";x\n";
        let edits = json!([{"range": at(0, 13, 0, 13), "newText": " "}]);
        let edits = from_lsp(text, &edits, Encoding::Utf16);
        let x = text.find('x').unwrap();
        assert_eq!(edits[0].0, x..x);
        assert_eq!(applied(text, &edits).unwrap(), "let é = \"😀\"; x\n");
        // The same column in bytes, for a server that said UTF-8.
        let edits = json!([{"range": at(0, 16, 0, 16), "newText": " "}]);
        assert_eq!(from_lsp(text, &edits, Encoding::Utf8)[0].0, x..x);
    }

    #[test]
    fn inserts_at_one_place_keep_their_order() {
        let text = "ab";
        let edits = json!([
            {"range": at(0, 1, 0, 1), "newText": "1"},
            {"range": at(0, 1, 0, 1), "newText": "2"},
            {"range": at(0, 1, 0, 2), "newText": "B"},
        ]);
        let edits = from_lsp(text, &edits, Encoding::Utf16);
        assert_eq!(applied(text, &edits).unwrap(), "a12B");
    }

    #[test]
    fn overlapping_edits_are_refused() {
        let edits = vec![(0..3, "x".to_string()), (2..4, "y".to_string())];
        assert!(splice("abcdef", &edits).is_none());
        assert!(applied("abcdef", &edits).is_none());
    }

    #[test]
    fn line_edits_touch_only_changed_lines() {
        let old = "a\nb\nc\nd\ne\n";
        let new = "a\nB\nc\nd\nE\nf\n";
        let edits = line_edits(old, new);
        assert_eq!(
            edits,
            vec![(2..4, "B\n".to_string()), (8..10, "E\nf\n".to_string())]
        );
        assert_eq!(applied(old, &edits).unwrap(), new);
        // A cursor on `d` stays on `d`.
        assert_eq!(
            shift(old.find('d').unwrap(), &edits),
            new.find('d').unwrap()
        );
        assert!(line_edits(old, old).is_empty());

        // In a reindented run of lines, on the same token.
        let old = "fn a(){\nlet x=1;\n}\n";
        let new = "fn a() {\n    let x = 1;\n}\n";
        let edits = line_edits(old, new);
        let carried = |token: &str| carry(old, old.find(token).unwrap(), &edits);
        assert_eq!(carried("x"), new.find('x').unwrap());
        assert_eq!(carried("=1"), new.find('=').unwrap());
        assert_eq!(carried("1;"), new.find('1').unwrap());
        assert_eq!(applied("x", &line_edits("x", "x\n")).unwrap(), "x\n");
        assert_eq!(applied("", &line_edits("", "y\n")).unwrap(), "y\n");
    }
}
