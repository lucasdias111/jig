//! Applying the unified diffs agents send, and paths as they name them.

use anyhow::{Context as _, Result};

pub(crate) fn relative(path: &str, directory: &str) -> String {
    let relative = path
        .strip_prefix(directory)
        .map(|rest| rest.trim_start_matches('/'))
        .unwrap_or(path);
    if relative.is_empty() {
        ".".to_string()
    } else {
        relative.to_string()
    }
}

/// Apply a unified diff to `old`. Each hunk is looked for where its header
/// says, and anywhere later in the file if the text has moved.
pub fn apply_diff(old: &str, diff: &str) -> Result<String> {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let mut out = String::new();
    let mut cursor = 0;
    let mut lines = diff.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let Some(header) = line.strip_prefix("@@ -") else {
            continue;
        };
        let start: usize = header
            .split([',', ' '])
            .next()
            .and_then(|n| n.parse().ok())
            .context("malformed hunk header")?;
        // Old and new text of the hunk, line by line.
        let (mut before, mut after): (Vec<String>, Vec<String>) = (vec![], vec![]);
        // Which side the last line went to, for "\ No newline at end of file".
        let mut last = (false, false);
        while let Some(&body) = lines.peek() {
            if body.starts_with("@@") {
                break;
            }
            lines.next();
            let (kind, text) = body.split_at(body.len().min(1));
            match kind {
                " " => {
                    before.push(text.into());
                    after.push(text.into());
                    last = (true, true);
                }
                "-" => {
                    before.push(text.into());
                    last = (true, false);
                }
                "+" => {
                    after.push(text.into());
                    last = (false, true);
                }
                "\\" => {
                    if last.0
                        && let Some(line) = before.last_mut()
                    {
                        line.truncate(line.trim_end_matches('\n').len());
                    }
                    if last.1
                        && let Some(line) = after.last_mut()
                    {
                        line.truncate(line.trim_end_matches('\n').len());
                    }
                }
                _ => {}
            }
        }
        let fits = |at: usize| {
            at + before.len() <= old_lines.len()
                && before
                    .iter()
                    .zip(&old_lines[at..])
                    .all(|(a, b)| a.trim_end_matches('\n') == b.trim_end_matches('\n'))
        };
        let expected = start.saturating_sub(1).max(cursor);
        let at = if fits(expected) {
            expected
        } else {
            (cursor..=old_lines.len())
                .find(|&at| fits(at))
                .context("the file changed since the agent read it")?
        };
        out.extend(old_lines[cursor..at].iter().copied());
        out.extend(after.iter().map(String::as_str));
        cursor = at + before.len();
    }
    out.extend(old_lines[cursor..].iter().copied());
    Ok(out)
}
