//! Ranking what's offered: the names in a file, and how well one matches
//! what was typed.

use super::is_name_char;

/// The names in `text`, with where each starts.
pub(super) fn words(text: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut chars = text.char_indices().peekable();
    std::iter::from_fn(move || {
        loop {
            let (start, first) = chars.next()?;
            if !is_name_char(first) {
                continue;
            }
            let mut end = start + first.len_utf8();
            while let Some(&(ix, c)) = chars.peek() {
                if !is_name_char(c) {
                    break;
                }
                end = ix + c.len_utf8();
                chars.next();
            }
            if !first.is_ascii_digit() {
                return Some((start, &text[start..end]));
            }
        }
    })
}

pub(super) const EXACT_PREFIX: u8 = 0;
pub(super) const PREFIX: u8 = 1;
pub(super) const SUBSEQUENCE: u8 = 2;

/// How well `candidate` matches what was `typed`, lower being better, or
/// `None` if it doesn't.
pub(super) fn match_tier(candidate: &str, typed: &str) -> Option<u8> {
    if candidate.starts_with(typed) {
        return Some(EXACT_PREFIX);
    }
    let fold = |s: &str| s.to_lowercase();
    let (candidate, typed) = (fold(candidate), fold(typed));
    if candidate.starts_with(&typed) {
        return Some(PREFIX);
    }
    let mut rest = candidate.chars();
    typed
        .chars()
        .all(|c| rest.any(|other| other == c))
        .then_some(SUBSEQUENCE)
}
