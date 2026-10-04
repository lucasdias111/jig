//! Which indentation a file already uses, so Tab keeps to it and the status
//! bar can say what it is.

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

#[cfg(test)]
mod tests {
    use super::*;

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
