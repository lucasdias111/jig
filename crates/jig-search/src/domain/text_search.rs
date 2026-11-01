//! Searching the text of every file in a project, for Find in Files.
//!
//! One result per matching line, in path then line order. Binary files,
//! huge files and anything `.gitignore` excludes are skipped.

use std::ops::Range;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use regex::{Regex, RegexBuilder};

/// Lines listed at most; the rest are only counted.
pub const MAX_RESULTS: usize = 2000;
/// Files larger than this are skipped: likely generated, and slow to show.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// Longest line text kept for display, in bytes.
const MAX_LINE_BYTES: usize = 240;
/// Shown before the match when a long line is cut on the left.
const LEAD_BYTES: usize = 48;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SearchOptions {
    pub match_case: bool,
    pub whole_word: bool,
    pub regex: bool,
}

/// One line with at least one match.
#[derive(Clone, Debug, PartialEq)]
pub struct LineMatch {
    /// Relative to the project root, `/`-separated.
    pub path: String,
    /// 0-based.
    pub line: usize,
    /// The line for display: leading whitespace removed, long lines cut
    /// around the first match.
    pub text: String,
    /// The matches within `text`, as byte ranges.
    pub highlights: Vec<Range<usize>>,
    /// The first match within the whole file, as a byte range.
    pub range: Range<usize>,
}

#[derive(Debug, Default, PartialEq)]
pub struct SearchResults {
    pub matches: Vec<LineMatch>,
    /// Matching lines, including any past [`MAX_RESULTS`].
    pub total_lines: usize,
    pub files: usize,
}

/// The regex a query and options search with. An invalid regex is an error
/// in words fit to show.
pub fn pattern(query: &str, options: SearchOptions) -> Result<Regex, String> {
    let mut source = if options.regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    if options.whole_word {
        source = format!(r"\b(?:{source})\b");
    }
    RegexBuilder::new(&source)
        .case_insensitive(!options.match_case)
        .multi_line(true)
        .build()
        .map_err(|error| match error {
            regex::Error::Syntax(_) => "Not a valid regular expression".to_string(),
            regex::Error::CompiledTooBig(_) => "That expression is too large".to_string(),
            _ => error.to_string(),
        })
}

/// Search `files` (relative to `root`) for `pattern`. Stops early, with what
/// it has, once `cancel` is set.
pub fn search(
    root: &Path,
    files: &[String],
    pattern: &Regex,
    cancel: &AtomicBool,
) -> SearchResults {
    let mut results = SearchResults::default();
    for path in files {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let full = root.join(path);
        if std::fs::metadata(&full).map_or(true, |m| m.len() > MAX_FILE_BYTES) {
            continue;
        }
        let Ok(bytes) = std::fs::read(&full) else {
            continue;
        };
        if bytes[..bytes.len().min(8192)].contains(&0) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let lines_before = results.total_lines;
        search_text(path, text, pattern, &mut results);
        if results.total_lines > lines_before {
            results.files += 1;
        }
    }
    results
}

fn search_text(path: &str, text: &str, pattern: &Regex, results: &mut SearchResults) {
    let mut found = pattern.find_iter(text).filter(|m| !m.is_empty()).peekable();
    let mut line = 0;
    let mut line_start = 0;
    while let Some(first) = found.next() {
        // Advance to the line holding this match.
        let before = &text[line_start..first.start()];
        if let Some(newline) = before.rfind('\n') {
            line += before.matches('\n').count();
            line_start += newline + 1;
        }
        let line_end = text[first.start()..]
            .find('\n')
            .map_or(text.len(), |newline| first.start() + newline);
        // Every match on this line is marked; a match running past the
        // line's end is marked up to it.
        let mut in_line = Vec::with_capacity(1);
        in_line.push(first.start()..first.end().min(line_end));
        while let Some(next) = found.peek()
            && next.start() < line_end
        {
            in_line.push(next.start()..next.end().min(line_end));
            found.next();
        }

        results.total_lines += 1;
        if results.matches.len() < MAX_RESULTS {
            let (display, highlights) =
                display_line(&text[line_start..line_end], line_start, &in_line);
            results.matches.push(LineMatch {
                path: path.to_string(),
                line,
                text: display,
                highlights,
                range: first.range(),
            });
        }
    }
}

/// The line of `text` holding `range`, as a result, e.g. for a reference a
/// language server found.
pub fn line_match(path: &str, text: &str, range: Range<usize>) -> LineMatch {
    let line_start = text[..range.start].rfind('\n').map_or(0, |ix| ix + 1);
    let line_end = text[range.start..]
        .find('\n')
        .map_or(text.len(), |ix| range.start + ix);
    let marked = range.start..range.end.min(line_end);
    let (text_shown, highlights) = display_line(&text[line_start..line_end], line_start, &[marked]);
    LineMatch {
        path: path.to_string(),
        line: text[..line_start].matches('\n').count(),
        text: text_shown,
        highlights,
        range,
    }
}

/// `line` (starting at byte `start` of the file) trimmed for display, with
/// the file ranges `found` moved into it.
fn display_line(line: &str, start: usize, found: &[Range<usize>]) -> (String, Vec<Range<usize>>) {
    let line = line.trim_end_matches('\r');
    let indent = line.len() - line.trim_start().len();
    let first = found[0].start - start;
    // Cut a long line so the first match shows with some context before it.
    let mut from = indent;
    if first > from + MAX_LINE_BYTES - LEAD_BYTES {
        from = floor_boundary(line, first - LEAD_BYTES);
    }
    let to = floor_boundary(line, (from + MAX_LINE_BYTES).min(line.len()));
    let prefix = if from > indent { "…" } else { "" };
    let suffix = if to < line.len() { "…" } else { "" };
    let text = format!("{prefix}{}{suffix}", &line[from..to]);
    let shift = |offset: usize| (offset - start).clamp(from, to) - from + prefix.len();
    let highlights = found
        .iter()
        .map(|range| shift(range.start)..shift(range.end))
        .filter(|range| !range.is_empty())
        .collect();
    (text, highlights)
}

fn floor_boundary(text: &str, mut index: usize) -> usize {
    while !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn find(text: &str, query: &str, options: SearchOptions) -> Vec<(usize, String, Vec<String>)> {
        let mut results = SearchResults::default();
        search_text(
            "a.rs",
            text,
            &pattern(query, options).unwrap(),
            &mut results,
        );
        results
            .matches
            .into_iter()
            .map(|m| {
                let marked = m
                    .highlights
                    .iter()
                    .map(|r| m.text[r.clone()].to_string())
                    .collect();
                (m.line, m.text, marked)
            })
            .collect()
    }

    const TEXT: &str = "fn main() {\n    let tab = Tab::new();\n    tabs.push(tab);\n}\n";

    #[test]
    fn one_result_per_line_with_every_match_marked() {
        assert_eq!(
            find(TEXT, "tab", SearchOptions::default()),
            [
                (
                    1,
                    "let tab = Tab::new();".into(),
                    vec!["tab".into(), "Tab".into()]
                ),
                (
                    2,
                    "tabs.push(tab);".into(),
                    vec!["tab".into(), "tab".into()]
                ),
            ]
        );
    }

    #[test]
    fn options_narrow_the_matches() {
        let case = SearchOptions {
            match_case: true,
            ..Default::default()
        };
        assert_eq!(find(TEXT, "Tab", case).len(), 1);
        let word = SearchOptions {
            whole_word: true,
            ..Default::default()
        };
        assert_eq!(
            find(TEXT, "tab", word)[1].2,
            ["tab"],
            "not the `tab` in `tabs`"
        );
        let regex = SearchOptions {
            regex: true,
            ..Default::default()
        };
        assert_eq!(find(TEXT, r"tabs?\.", regex)[0].2, ["tabs."]);
        assert_eq!(
            find(TEXT, "tabs?.", SearchOptions::default()).len(),
            0,
            "literal"
        );
    }

    #[test]
    fn bad_regex_is_reported() {
        let regex = SearchOptions {
            regex: true,
            ..Default::default()
        };
        assert_eq!(
            pattern("(", regex).unwrap_err(),
            "Not a valid regular expression"
        );
        assert!(pattern("(", SearchOptions::default()).is_ok());
    }

    #[test]
    fn range_points_into_the_file() {
        let mut results = SearchResults::default();
        let regex = pattern("push", SearchOptions::default()).unwrap();
        search_text("a.rs", TEXT, &regex, &mut results);
        let range = results.matches[0].range.clone();
        assert_eq!(&TEXT[range], "push");
    }

    #[test]
    fn long_lines_are_cut_around_the_match() {
        let line = format!("{}needle{}", "x".repeat(500), "y".repeat(500));
        let found = find(&line, "needle", SearchOptions::default());
        let (_, text, marked) = &found[0];
        assert!(text.starts_with('…') && text.ends_with('…'));
        assert!(text.len() <= MAX_LINE_BYTES + 2 * '…'.len_utf8());
        assert_eq!(marked, &["needle"]);
    }

    #[test]
    fn search_skips_binary_and_counts_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), TEXT).unwrap();
        fs::write(dir.path().join("b.bin"), b"tab\0tab").unwrap();
        fs::write(dir.path().join("c.md"), "tab here\n").unwrap();
        let files = ["a.rs", "b.bin", "c.md"].map(String::from);
        let regex = pattern("tab", SearchOptions::default()).unwrap();
        let results = search(dir.path(), &files, &regex, &AtomicBool::new(false));
        assert_eq!(results.files, 2);
        assert_eq!(results.total_lines, 3);
        assert_eq!(results.matches[2].path, "c.md");
    }
}
