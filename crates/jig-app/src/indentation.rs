//! Which indentation a file already uses, so Tab keeps to it and the status
//! bar can say what it is.

use std::ops::Range;

use gpui_kit::component::input::TabSize;

/// Used for new files and files with no indented lines.
pub const DEFAULT: TabSize = TabSize {
    tab_size: 4,
    hard_tabs: false,
};

/// How far a tab reaches when a file is indented with tabs.
const TAB_WIDTH: usize = 4;

/// Lines past these are not looked at; the top of a file is enough to tell.
const MAX_LINES: usize = 10_000;

/// Guess `text`'s indentation: tabs if most indented lines start with one,
/// otherwise spaces, as many as the most common step from one line's
/// indentation to the next.
pub fn detect(text: &str) -> TabSize {
    let mut tab_lines = 0;
    let mut space_lines = 0;
    // How often each step of 1–8 spaces appears between consecutive lines.
    let mut steps = [0usize; 9];
    let mut previous = 0;
    for line in text.lines().take(MAX_LINES) {
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with('\t') {
            tab_lines += 1;
            previous = 0;
            continue;
        }
        let spaces = line.len() - line.trim_start_matches(' ').len();
        if spaces > 0 {
            space_lines += 1;
        }
        let step = spaces.abs_diff(previous);
        if step < steps.len() {
            steps[step] += 1;
        }
        previous = spaces;
    }

    if tab_lines > space_lines {
        return TabSize {
            tab_size: TAB_WIDTH,
            hard_tabs: true,
        };
    }
    // A step of one space is more often a block comment's ` * ` than an
    // indent.
    let common = (2..steps.len())
        .filter(|&step| steps[step] > 0)
        .max_by_key(|&step| (steps[step], std::cmp::Reverse(step)));
    match common {
        Some(tab_size) => TabSize {
            tab_size,
            hard_tabs: false,
        },
        None => DEFAULT,
    }
}

/// As the status bar shows it: `"Spaces: 4"` or `"Tab Size: 4"`.
pub fn label(tab: TabSize) -> String {
    if tab.hard_tabs {
        format!("Tab Size: {}", tab.tab_size)
    } else {
        format!("Spaces: {}", tab.tab_size)
    }
}

/// The indent guide of the block `row` is in, as the rows it runs down and
/// its column in spaces, with guides every `tab_size` columns as the editor
/// draws them. On a line that opens a block (the next line is indented
/// further) it's the guide of that block. `None` at the top level.
pub fn active_guide(text: &str, row: usize, tab_size: usize) -> Option<(Range<usize>, usize)> {
    let tab_size = tab_size.max(1);
    // Each line's indentation, `None` for blank lines.
    let indents: Vec<Option<usize>> = text
        .split('\n')
        .map(|line| {
            (!line.trim().is_empty()).then(|| {
                line.chars()
                    .map_while(|c| match c {
                        ' ' => Some(1),
                        '\t' => Some(tab_size),
                        _ => None,
                    })
                    .sum()
            })
        })
        .collect();
    let here = indents.get(row).copied()?;
    let below = indents[row + 1..].iter().find_map(|indent| *indent);
    let above = indents[..row].iter().rev().find_map(|indent| *indent);
    // A blank line belongs to the deeper of the blocks around it.
    let indent = here.or_else(|| above.max(below))?;
    let opens_block = here.is_some() && below.is_some_and(|below| below > indent);
    let column = if opens_block {
        indent / tab_size * tab_size
    } else {
        indent.checked_sub(1)? / tab_size * tab_size
    };
    let inside = |row: usize| indents[row].is_none_or(|indent| indent > column);
    let mut start = if opens_block { row + 1 } else { row };
    while start > 0 && !opens_block && inside(start - 1) {
        start -= 1;
    }
    let mut end = start;
    while end < indents.len() && inside(end) {
        end += 1;
    }
    // Blank lines at either end aren't part of the block.
    while start < end && indents[start].is_none() {
        start += 1;
    }
    while end > start && indents[end - 1].is_none() {
        end -= 1;
    }
    (start < end).then_some((start..end, column))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_active_guide_is_the_block_the_cursor_is_in() {
        let text = "fn a() {\n    if x {\n        y();\n\n        z();\n    }\n}\n";
        // Inside `if`: the guide at column 4, down its body.
        assert_eq!(active_guide(text, 2, 4), Some((2..5, 4)));
        // A blank line inside it too.
        assert_eq!(active_guide(text, 3, 4), Some((2..5, 4)));
        // On `if x {`, which opens that block: the same guide.
        assert_eq!(active_guide(text, 1, 4), Some((2..5, 4)));
        // On `}` closing it: the function's guide.
        assert_eq!(active_guide(text, 5, 4), Some((1..6, 0)));
        // On `fn a() {`: the function's guide.
        assert_eq!(active_guide(text, 0, 4), Some((1..6, 0)));
        // The closing `}` of the function is at the top level.
        assert_eq!(active_guide(text, 6, 4), None);
    }

    #[test]
    fn tabs_count_as_the_tab_size() {
        let text = "a:\n\tb\n\tc\nd\n";
        assert_eq!(active_guide(text, 2, 4), Some((1..3, 0)));
    }

    fn detected(text: &str) -> (usize, bool) {
        let tab = detect(text);
        (tab.tab_size, tab.hard_tabs)
    }

    #[test]
    fn spaces() {
        assert_eq!(
            detected("fn a() {\n    if b {\n        c();\n    }\n}\n"),
            (4, false)
        );
        assert_eq!(detected("a:\n  b:\n    c: 1\n  d: 2\n"), (2, false));
    }

    #[test]
    fn tabs() {
        assert_eq!(
            detected("func a() {\n\tif b {\n\t\tc()\n\t}\n}\n"),
            (4, true)
        );
    }

    #[test]
    fn block_comments_do_not_count_as_one_space() {
        let text = "/**\n * Docs.\n * More.\n */\nfn a() {\n    b();\n}\n";
        assert_eq!(detected(text), (4, false));
    }

    #[test]
    fn nothing_indented_uses_the_default() {
        assert_eq!(detected(""), (4, false));
        assert_eq!(detected("a\nb\n"), (4, false));
    }
}
