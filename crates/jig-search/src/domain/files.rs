//! Go to File's list: the recent files, then the rest, or the files a
//! query fuzzy-matches, each marked recent or not.

use std::collections::HashSet;

use jig_commands::fuzzy;

/// A listed file.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// Relative to the project root, `/`-separated.
    pub path: String,
    /// Byte offsets of the characters the query matched.
    pub positions: Vec<usize>,
    pub recent: bool,
}

/// The rows for `query`, at most `max`: with nothing typed, the `recent`
/// files (most recent first) then the rest of `files`; else the files the
/// query matches, best first.
pub fn rows(files: &[String], recent: &[String], query: &str, max: usize) -> Vec<Row> {
    let recent_set: HashSet<&str> = recent.iter().map(String::as_str).collect();
    if query.trim().is_empty() {
        let row = |path: &String, recent| Row {
            path: path.clone(),
            positions: Vec::new(),
            recent,
        };
        return recent
            .iter()
            .map(|path| row(path, true))
            .chain(
                files
                    .iter()
                    .filter(|path| !recent_set.contains(path.as_str()))
                    .map(|path| row(path, false)),
            )
            .take(max)
            .collect();
    }
    fuzzy::match_paths(files.iter().map(String::as_str), query, max)
        .into_iter()
        .map(|m| {
            let path = &files[m.index];
            Row {
                recent: recent_set.contains(path.as_str()),
                path: path.clone(),
                positions: m.positions,
            }
        })
        .collect()
}

/// `12345` as `12,345`.
pub fn group_digits(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests;
