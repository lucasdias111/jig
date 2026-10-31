//! Fuzzy matching of project-relative paths, for Go to File.
//!
//! The query's characters must appear in the path in order, ignoring case
//! and spaces. Matches inside the file name beat matches spread over the
//! folders, and among those, runs of consecutive characters, characters at
//! the start of a word, and shorter paths rank higher.

/// One path that matched: how well, and which characters to highlight.
#[derive(Clone, Debug, PartialEq)]
pub struct PathMatch {
    /// Index into the paths given to [`match_paths`].
    pub index: usize,
    /// Higher is better.
    pub score: i32,
    /// Byte offsets of the matched characters in the path.
    pub positions: Vec<usize>,
}

/// A match entirely inside the file name gets this much on top.
const NAME_BONUS: i32 = 1000;
const START_BONUS: i32 = 12;
const BOUNDARY_BONUS: i32 = 8;
const CONSECUTIVE_BONUS: i32 = 6;
/// Per character skipped between two matched ones; capped per gap so one
/// long folder name doesn't sink a match.
const GAP_PENALTY: i32 = 1;
const MAX_GAP_PENALTY: i32 = 8;

/// The paths matching `query`, best first. Ties keep the given order.
pub fn match_paths<'a>(
    paths: impl IntoIterator<Item = &'a str>,
    query: &str,
    limit: usize,
) -> Vec<PathMatch> {
    let query: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(fold)
        .collect();
    let mut matches: Vec<PathMatch> = paths
        .into_iter()
        .enumerate()
        .filter_map(|(index, path)| {
            let (score, positions) = score_path(path, &query)?;
            Some(PathMatch {
                index,
                score,
                positions,
            })
        })
        .collect();
    // Stable, so equal scores keep the walk's order.
    matches.sort_by_key(|m| std::cmp::Reverse(m.score));
    matches.truncate(limit);
    matches
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

/// The score and matched byte offsets of `query` in `path`, or `None` when
/// it isn't a subsequence of it.
fn score_path(path: &str, query: &[char]) -> Option<(i32, Vec<usize>)> {
    if query.is_empty() {
        return Some((0, Vec::new()));
    }
    let chars: Vec<(usize, char)> = path.char_indices().collect();
    let name_start = chars
        .iter()
        .rposition(|&(_, c)| c == '/')
        .map_or(0, |slash| slash + 1);
    let shortness = -(chars.len() as i32) / 4;
    // Try the file name alone first: `tabs` should land on `tabs.rs`, not
    // on the first t, a, b and s found in the folders above it.
    if let Some((score, positions)) = score_from(&chars, name_start, query) {
        let exact_prefix = chars.len() - name_start >= query.len()
            && chars[name_start..]
                .iter()
                .zip(query)
                .all(|(&(_, c), &q)| fold(c) == q);
        let prefix_bonus = if exact_prefix { NAME_BONUS / 2 } else { 0 };
        return Some((NAME_BONUS + prefix_bonus + score + shortness, positions));
    }
    score_from(&chars, 0, query).map(|(score, positions)| (score + shortness, positions))
}

/// Match `query` in `chars[start..]`, taking each character at the first
/// word start ahead of it when there is one before the next forced choice,
/// otherwise at its first occurrence.
fn score_from(chars: &[(usize, char)], start: usize, query: &[char]) -> Option<(i32, Vec<usize>)> {
    // Quick reject, and the latest position each query character may take
    // so the rest still fits.
    let mut latest = vec![0; query.len()];
    let mut at = chars.len();
    for (qi, &q) in query.iter().enumerate().rev() {
        at = (start..at).rev().find(|&i| fold(chars[i].1) == q)?;
        latest[qi] = at;
    }

    let mut score = 0;
    let mut positions = Vec::with_capacity(query.len());
    let mut previous: Option<usize> = None;
    let mut from = start;
    for (qi, &q) in query.iter().enumerate() {
        let candidates = (from..=latest[qi]).filter(|&i| fold(chars[i].1) == q);
        let mut first = None;
        let mut chosen = None;
        for i in candidates {
            first.get_or_insert(i);
            // Keep a run going rather than jumping to a word start.
            if previous.is_some_and(|p| i == p + 1) || is_boundary(chars, i, start) {
                chosen = Some(i);
                break;
            }
        }
        let i = chosen.or(first)?;
        if i == start {
            score += START_BONUS;
        } else if is_boundary(chars, i, start) {
            score += BOUNDARY_BONUS;
        }
        match previous {
            Some(p) if i == p + 1 => score += CONSECUTIVE_BONUS,
            Some(p) => score -= ((i - p - 1) as i32 * GAP_PENALTY).min(MAX_GAP_PENALTY),
            None => score -= ((i - start) as i32 * GAP_PENALTY).min(MAX_GAP_PENALTY),
        }
        positions.push(chars[i].0);
        previous = Some(i);
        from = i + 1;
    }
    Some((score, positions))
}

/// The start of a word: after a separator, or an uppercase letter after a
/// lowercase one (`camelCase`).
fn is_boundary(chars: &[(usize, char)], i: usize, start: usize) -> bool {
    if i == start {
        return true;
    }
    let before = chars[i - 1].1;
    let c = chars[i].1;
    matches!(before, '/' | '_' | '-' | '.' | ' ') || (before.is_lowercase() && c.is_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn best<'a>(paths: &[&'a str], query: &str) -> Vec<&'a str> {
        match_paths(paths.iter().copied(), query, usize::MAX)
            .into_iter()
            .map(|m| paths[m.index])
            .collect()
    }

    const PATHS: &[&str] = &[
        "crates/jig-app/src/workspace.rs",
        "crates/jig-app/src/workspace/tabs.rs",
        "crates/jig-app/src/workspace/sidebar.rs",
        "crates/jig-app/src/file_tree.rs",
        "crates/jig-commands/src/palette.rs",
        "Cargo.toml",
        "assets/themes/ember.json",
    ];

    #[test]
    fn empty_query_matches_everything_in_order() {
        assert_eq!(best(PATHS, ""), PATHS);
        assert_eq!(best(PATHS, "   "), PATHS);
    }

    #[test]
    fn non_subsequences_are_left_out() {
        assert!(best(PATHS, "zzz").is_empty());
        assert_eq!(best(PATHS, "ember"), ["assets/themes/ember.json"]);
    }

    #[test]
    fn file_names_beat_folders() {
        assert_eq!(
            best(PATHS, "tabs")[0],
            "crates/jig-app/src/workspace/tabs.rs"
        );
        assert_eq!(best(PATHS, "work")[0], "crates/jig-app/src/workspace.rs");
        assert_eq!(best(PATHS, "cargo")[0], "Cargo.toml");
    }

    #[test]
    fn folders_narrow_the_search() {
        assert_eq!(
            best(PATHS, "ws/tabs")[0],
            "crates/jig-app/src/workspace/tabs.rs"
        );
        assert_eq!(
            best(PATHS, "cmd pal")[0],
            "crates/jig-commands/src/palette.rs"
        );
    }

    #[test]
    fn word_starts_and_case() {
        assert_eq!(best(PATHS, "FT")[0], "crates/jig-app/src/file_tree.rs");
        assert_eq!(best(&["fooBar.rs", "fobar.rs"], "fb")[0], "fooBar.rs");
    }

    #[test]
    fn positions_mark_the_matched_characters() {
        let m = &match_paths(["src/file_tree.rs"], "ftr", 10)[0];
        let path = "src/file_tree.rs";
        let picked: String = m.positions.iter().map(|&i| &path[i..i + 1]).collect();
        assert_eq!(picked, "ftr");
        assert_eq!(m.positions, [4, 9, 10], "the word start, then a run");
    }

    #[test]
    fn non_ascii_paths_use_byte_offsets() {
        let path = "données/résumé.md";
        let m = &match_paths([path], "rés", 10)[0];
        for &i in &m.positions {
            assert!(path.is_char_boundary(i));
        }
        assert_eq!(&path[m.positions[1]..m.positions[2]], "é");
    }

    #[test]
    fn limit_keeps_the_best() {
        assert_eq!(match_paths(PATHS.iter().copied(), "rs", 2).len(), 2);
    }
}
