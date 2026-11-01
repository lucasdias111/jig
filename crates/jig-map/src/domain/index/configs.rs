//! What a project's own files say about it, read from their text: what a
//! README says its folder is for, and the path aliases a tsconfig sets.

use std::path::Path;

use super::{Alias, normalize};

/// What a README says its folder is for: its first paragraph's first
/// sentence, past its title and badges.
pub fn readme_purpose(text: &str) -> Option<String> {
    let paragraph: Vec<&str> = text
        .lines()
        .map(str::trim)
        .skip_while(|line| {
            line.is_empty()
                || line.starts_with('#')
                || line.starts_with('[')
                || line.starts_with('!')
                || line.starts_with('<')
                || line.starts_with("===")
                || line.starts_with("---")
        })
        .take_while(|line| !line.is_empty())
        .collect();
    let text = paragraph.join(" ");
    let end = text
        .match_indices(". ")
        .map(|(at, _)| at + 1)
        .find(|&at| at > 12)
        .unwrap_or(text.len());
    let purpose: String = text[..end].replace(['*', '`', '_'], "");
    (!purpose.is_empty()).then_some(purpose)
}

/// The aliases tsconfig file `config` (relative to the project, its
/// text `text`) sets: each `compilerOptions.paths` entry ending `/*`, from
/// its `baseUrl`.
pub fn aliases_in(config: &Path, text: &str) -> Vec<Alias> {
    let mut found = Vec::new();
    {
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&without_comments(text)) else {
            return found;
        };
        let options = &json["compilerOptions"];
        let dir = config.parent().map(Path::to_path_buf).unwrap_or_default();
        let base = normalize(&dir.join(options["baseUrl"].as_str().unwrap_or(".")));
        let Some(paths) = options["paths"].as_object() else {
            return found;
        };
        for (pattern, targets) in paths {
            let (Some(prefix), Some(target)) = (
                pattern.strip_suffix('*'),
                targets
                    .get(0)
                    .and_then(|t| t.as_str())
                    .and_then(|t| t.strip_suffix('*')),
            ) else {
                continue;
            };
            let alias = Alias {
                dir: dir.clone(),
                prefix: prefix.to_string(),
                target: normalize(&base.join(target)),
            };
            if !found.contains(&alias) {
                found.push(alias);
            }
        }
    }
    found
}

/// JSON with comments (as tsconfig files have them) without them, and
/// without trailing commas.
fn without_comments(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                out.extend(chars.next());
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
            }
            _ => out.push(c),
        }
    }
    // `,}` and `,]`.
    let mut cleaned = String::new();
    let bytes: Vec<char> = out.chars().collect();
    for (ix, &c) in bytes.iter().enumerate() {
        if c == ',' {
            let next = bytes[ix + 1..].iter().find(|c| !c.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        cleaned.push(c);
    }
    cleaned
}
