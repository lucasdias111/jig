//! What a change actually changed, line by line, so a preview can mark just
//! the new lines instead of the whole region it replaced.

use std::ops::Range;

/// The lines `new` adds to `old`, and the lines it drops.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LineDiff {
    /// Byte ranges in `new` of each run of added lines.
    pub added: Vec<Range<usize>>,
    /// Each run of removed lines from `old`.
    pub removed: Vec<String>,
}

/// Regions with more lines than this on either side aren't diffed line by
/// line (the table grows with the product); everything counts as changed.
const MAX_LINES: usize = 3000;

pub fn line_diff(old: &str, new: &str) -> LineDiff {
    let a: Vec<&str> = old.split_inclusive('\n').collect();
    let b: Vec<&str> = new.split_inclusive('\n').collect();
    if a.len() > MAX_LINES || b.len() > MAX_LINES {
        return LineDiff {
            added: (!new.is_empty())
                .then_some(0..new.len())
                .into_iter()
                .collect(),
            removed: (!old.is_empty())
                .then(|| old.to_string())
                .into_iter()
                .collect(),
        };
    }
    // A line matches regardless of its final newline, so the last line of a
    // region doesn't count as changed just because one side ends the file.
    let same = |i: usize, j: usize| a[i].trim_end_matches('\n') == b[j].trim_end_matches('\n');
    // lcs[i][j]: longest common subsequence of a[i..] and b[j..].
    let mut lcs = vec![vec![0u32; b.len() + 1]; a.len() + 1];
    for i in (0..a.len()).rev() {
        for j in (0..b.len()).rev() {
            lcs[i][j] = if same(i, j) {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut diff = LineDiff::default();
    let (mut i, mut j, mut offset) = (0, 0, 0);
    let mut removing = String::new();
    let mut adding: Option<Range<usize>> = None;
    let flush = |removing: &mut String, adding: &mut Option<Range<usize>>, diff: &mut LineDiff| {
        if !removing.is_empty() {
            diff.removed.push(std::mem::take(removing));
        }
        if let Some(range) = adding.take() {
            diff.added.push(range);
        }
    };
    while i < a.len() || j < b.len() {
        if i < a.len() && j < b.len() && same(i, j) {
            flush(&mut removing, &mut adding, &mut diff);
            offset += b[j].len();
            i += 1;
            j += 1;
        } else if j < b.len() && (i == a.len() || lcs[i][j + 1] >= lcs[i + 1][j]) {
            let end = offset + b[j].len();
            adding = Some(adding.map_or(offset..end, |range| range.start..end));
            offset = end;
            j += 1;
        } else {
            removing.push_str(a[i]);
            i += 1;
        }
    }
    flush(&mut removing, &mut adding, &mut diff);
    diff
}

/// The smallest range of `old` to replace, and its replacement, to get
/// `new`: everything between their common start and common end.
pub fn changed_range<'a>(old: &str, new: &'a str) -> (Range<usize>, &'a str) {
    let mut prefix = old
        .bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(prefix) || !new.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let max_suffix = old.len().min(new.len()) - prefix;
    let mut suffix = old
        .bytes()
        .rev()
        .zip(new.bytes().rev())
        .take(max_suffix)
        .take_while(|(a, b)| a == b)
        .count();
    while !old.is_char_boundary(old.len() - suffix) || !new.is_char_boundary(new.len() - suffix) {
        suffix -= 1;
    }
    (prefix..old.len() - suffix, &new[prefix..new.len() - suffix])
}

/// What a preview shows for replacing `old` with `new` at `start` in the
/// buffer: the edit to make (only the part that differs), the new lines to
/// highlight (buffer offsets, after the edit) and the removed lines.
pub struct PreviewEdit<'a> {
    pub range: Range<usize>,
    pub replacement: &'a str,
    pub highlights: Vec<Range<usize>>,
    pub removed: String,
}

pub fn preview_edit<'a>(old: &str, new: &'a str, start: usize) -> PreviewEdit<'a> {
    let (changed, replacement) = changed_range(old, new);
    // Diff whole lines, so a line that only gained a few characters shows as
    // that one line changed.
    let line_start = old[..changed.start].rfind('\n').map_or(0, |ix| ix + 1);
    let old_end = old[changed.end..]
        .find('\n')
        .map_or(old.len(), |ix| changed.end + ix + 1);
    let new_end = old_end - changed.end + changed.start + replacement.len();
    let diff = line_diff(&old[line_start..old_end], &new[line_start..new_end]);
    PreviewEdit {
        range: start + changed.start..start + changed.end,
        replacement,
        highlights: diff
            .added
            .into_iter()
            .map(|range| start + line_start + range.start..start + line_start + range.end)
            .collect(),
        removed: diff.removed.join("⋯\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_doc_comments_are_the_only_change() {
        let old = "fn a() {}\n\nfn b() {}\n";
        let new = "/// A.\nfn a() {}\n\n/// B.\nfn b() {}\n";
        let diff = line_diff(old, new);
        assert_eq!(diff.removed, Vec::<String>::new());
        let added: Vec<&str> = diff.added.iter().map(|r| &new[r.clone()]).collect();
        assert_eq!(added, ["/// A.\n", "/// B.\n"]);
    }

    #[test]
    fn a_changed_line_is_removed_and_added() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\n";
        let diff = line_diff(old, new);
        assert_eq!(diff.removed, ["b\n"]);
        assert_eq!(diff.added, vec![2..4]);
    }

    #[test]
    fn a_missing_final_newline_isnt_a_change() {
        let diff = line_diff("a\nb", "/// x\na\nb\n");
        assert_eq!(diff.removed, Vec::<String>::new());
        assert_eq!(diff.added, vec![0..6]);
    }

    #[test]
    fn preview_edits_and_highlights_only_what_changed() {
        let buffer_offset = 100;
        let old = "fn a() {}\n\nfn b() {}";
        let new = "/// A.\nfn a() {}\n\n/// B.\nfn b() {}";
        let edit = preview_edit(old, new, buffer_offset);
        // The edit spans from the first to the last difference, no further.
        assert_eq!(edit.range, 100..110);
        assert_eq!(edit.replacement, "/// A.\nfn a() {}\n\n/// B.");
        assert_eq!(edit.removed, "");
        // Offsets are in the buffer after the edit.
        assert_eq!(edit.highlights, [100..107, 118..125]);
    }

    #[test]
    fn unchanged_text_gives_an_empty_preview() {
        let edit = preview_edit("same\n", "same\n", 0);
        assert_eq!(edit.range, 5..5);
        assert!(edit.replacement.is_empty() && edit.highlights.is_empty());
    }

    #[test]
    fn changed_range_is_the_part_in_between() {
        assert_eq!(
            changed_range("fn a() {}\n", "/// A.\nfn a() {}\n"),
            (0..0, "/// A.\n")
        );
        assert_eq!(changed_range("abcdef", "abXYef"), (2..4, "XY"));
        assert_eq!(changed_range("aaa", "aaaa"), (3..3, "a"));
        // Never splits a character.
        assert_eq!(changed_range("é", "è"), (0..2, "è"));
    }
}
