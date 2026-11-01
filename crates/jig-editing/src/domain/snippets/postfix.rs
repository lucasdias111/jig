//! Postfix snippets: the expression one wraps, and a name guessed for it.

use std::ops::Range;

/// The expression a postfix snippet typed at `dot` (the `.`) wraps: back
/// over names, calls, indexing, strings and `.`/`::` chains.
pub fn receiver(text: &str, dot: usize) -> Option<Range<usize>> {
    let bytes = text.as_bytes();
    if bytes.get(dot) != Some(&b'.') {
        return None;
    }
    // Where the expression starts, as far as it's been read.
    let mut start = dot;
    let mut at = dot;
    loop {
        // Calls, indexing, strings, `!` and `?`, right to left.
        while at > 0 {
            match bytes[at - 1] {
                close @ (b')' | b']' | b'}') => {
                    let open = match close {
                        b')' => b'(',
                        b']' => b'[',
                        _ => b'{',
                    };
                    at = matching_open(&bytes[..at], open, close)?;
                }
                quote @ (b'"' | b'\'' | b'`') => at = matching_quote(&bytes[..at - 1], quote)?,
                b'!' | b'?'
                    if at > 1
                        && (is_name_byte(bytes[at - 2]) || b")]".contains(&bytes[at - 2])) =>
                {
                    at -= 1
                }
                _ => break,
            }
        }
        while at > 0 && is_name_byte(bytes[at - 1]) {
            at -= 1;
        }
        if at == start || (at < start && bytes[at..start].iter().all(|b| b".?:".contains(b))) {
            break;
        }
        start = at;
        // On over a chain: `a.b`, `a?.b`, `A::b`.
        if at > 1 && bytes[at - 1] == b'.' {
            at -= 1;
            if bytes[at - 1] == b'?' {
                at -= 1;
            }
        } else if at > 2 && &bytes[at - 2..at] == b"::" {
            at -= 2;
        } else {
            break;
        }
    }
    (start < dot && text.is_char_boundary(start)).then_some(start..dot)
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

/// Where the bracket closing `bytes` opens.
fn matching_open(bytes: &[u8], open: u8, close: u8) -> Option<usize> {
    let mut depth = 0;
    for ix in (0..bytes.len()).rev() {
        if bytes[ix] == close {
            depth += 1;
        } else if bytes[ix] == open {
            depth -= 1;
            if depth == 0 {
                return Some(ix);
            }
        }
    }
    None
}

/// Where the string whose closing `quote` follows `bytes` opens.
fn matching_quote(bytes: &[u8], quote: u8) -> Option<usize> {
    (0..bytes.len())
        .rev()
        .find(|&ix| bytes[ix] == quote && (ix == 0 || bytes[ix - 1] != b'\\'))
}

/// Calls whose result is best named after what they were called on:
/// `user.clone()` is a `user`, `Vec::new()` a `vec`.
const PASS_THROUGH: &[&str] = &[
    "new",
    "default",
    "from",
    "clone",
    "unwrap",
    "expect",
    "to_owned",
    "iter",
    "into_iter",
    "collect",
    "await",
    "build",
    "parse",
    "as_ref",
    "borrow",
    "ok",
    "lock",
    "read",
    "get",
];
const VERB_PREFIXES: &[&str] = &[
    "get", "fetch", "load", "find", "read", "create", "make", "build", "compute", "to", "as",
    "into",
];
const RESERVED: &[&str] = &[
    "self", "this", "type", "match", "fn", "let", "const", "var", "class", "def", "for", "if",
    "in", "is", "return", "new", "loop", "while", "impl", "mod", "use", "struct", "enum",
];

/// A variable name for `expr`: `getUser(id)` is `user`, `HttpClient::new()`
/// is `http_client` (or `httpClient`, depending on `language`).
pub fn name_for(expr: &str, language: &str) -> String {
    // Only what's outside brackets names the result.
    let outer = outer_text(expr);
    let mut names: Vec<&str> = outer
        .split(['.', ':', '!', '?'])
        .map(|s| s.trim_matches(|c: char| !(c.is_alphanumeric() || c == '_')))
        .filter(|s| !s.is_empty() && !s.starts_with(|c: char| c.is_ascii_digit()))
        .collect();
    while names.len() > 1 && PASS_THROUGH.contains(&names[names.len() - 1].to_lowercase().as_str())
    {
        names.pop();
    }
    let Some(last) = names.last() else {
        return "value".into();
    };
    let words = words_of(last);
    let words = match words.split_first() {
        Some((first, rest)) if !rest.is_empty() && VERB_PREFIXES.contains(&first.as_str()) => {
            rest.to_vec()
        }
        _ => words,
    };
    let name = match language {
        "rust" | "python" => words.join("_"),
        _ => words
            .iter()
            .enumerate()
            .map(|(ix, word)| {
                if ix == 0 {
                    word.clone()
                } else {
                    let mut chars = word.chars();
                    chars
                        .next()
                        .map(|c| c.to_uppercase().chain(chars).collect())
                        .unwrap_or_default()
                }
            })
            .collect(),
    };
    if name.is_empty() || RESERVED.contains(&name.as_str()) {
        "value".into()
    } else {
        name
    }
}

/// `expr` without anything inside brackets or quotes.
fn outer_text(expr: &str) -> String {
    let mut depth = 0usize;
    let mut quote = None;
    let mut out = String::new();
    for c in expr.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, '"' | '\'' | '`') => quote = Some(c),
            (None, '(' | '[' | '{') => depth += 1,
            (None, ')' | ']' | '}') => depth = depth.saturating_sub(1),
            (None, c) if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// The lowercase words of a snake_case or camelCase name.
fn words_of(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let chars: Vec<char> = name.chars().collect();
    for (ix, &c) in chars.iter().enumerate() {
        if c == '_' || c == '$' {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
            continue;
        }
        let starts_word = c.is_uppercase()
            && !word.is_empty()
            && (chars[ix - 1].is_lowercase()
                || chars.get(ix + 1).is_some_and(|next| next.is_lowercase()));
        if starts_word {
            words.push(std::mem::take(&mut word));
        }
        word.extend(c.to_lowercase());
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}
