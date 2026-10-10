//! Which bracket pairs with the one at the cursor, found by scanning the
//! text around it. Brackets in strings and comments don't count, so a `"("`
//! doesn't throw the count off.

/// How far either side of the cursor to look for the other bracket.
const REACH: usize = 64 * 1024;

/// The bracket next to `cursor` (just before it first, then just after)
/// and the one it pairs with, as their byte offsets, opening first.
/// `line_comment` is what starts a comment in the file's language.
pub fn matching(text: &str, cursor: usize, line_comment: Option<&str>) -> Option<(usize, usize)> {
    let cursor = cursor.min(text.len());
    let bytes = text.as_bytes();
    let candidates = [cursor.checked_sub(1), Some(cursor)];
    let at = candidates
        .into_iter()
        .flatten()
        .find(|&at| bytes.get(at).is_some_and(|&b| is_bracket(b)))?;
    // Start scanning at a line start, where code is most likely outside a
    // string or comment.
    let from = text[..at.saturating_sub(REACH)]
        .rfind('\n')
        .map_or(0, |newline| newline + 1);
    let to = (at + REACH).min(text.len());
    let mut open: Vec<usize> = Vec::new();
    let mut found = None;
    for ix in code_brackets(&bytes[..to], from, line_comment) {
        if is_opening(bytes[ix]) {
            open.push(ix);
            continue;
        }
        let Some(start) = open.pop() else {
            continue;
        };
        let pairs = closing_for(bytes[start]) == bytes[ix];
        if start == at || ix == at {
            found = pairs.then_some((start, ix));
            break;
        }
    }
    found
}

/// Offsets of the brackets in `bytes[from..]` outside strings and comments.
fn code_brackets(bytes: &[u8], from: usize, line_comment: Option<&str>) -> Vec<usize> {
    let line_comment = line_comment.map(str::as_bytes);
    // C-style block comments go with `//` line comments.
    let block_comments = line_comment == Some(b"//".as_slice());
    let mut found = Vec::new();
    let mut ix = from;
    while ix < bytes.len() {
        let rest = &bytes[ix..];
        if line_comment.is_some_and(|token| rest.starts_with(token)) {
            ix = skip_past(bytes, ix, b"\n");
        } else if block_comments && rest.starts_with(b"/*") {
            ix = skip_past(bytes, ix + 2, b"*/");
        } else if rest[0] == b'"' || rest[0] == b'`' {
            ix = skip_string(bytes, ix);
        } else if let Some(len) = char_literal(rest) {
            ix += len;
        } else {
            if is_bracket(rest[0]) {
                found.push(ix);
            }
            ix += 1;
        }
    }
    found
}

/// Just past the next `end` at or after `ix`, or the end of `bytes`.
fn skip_past(bytes: &[u8], ix: usize, end: &[u8]) -> usize {
    bytes[ix..]
        .windows(end.len())
        .position(|window| window == end)
        .map_or(bytes.len(), |at| ix + at + end.len())
}

/// Just past the string opening at `ix`. A string runs to its closing
/// quote, skipping escaped ones; an unclosed `"` ends at the line's end.
fn skip_string(bytes: &[u8], ix: usize) -> usize {
    let quote = bytes[ix];
    let mut at = ix + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 2,
            b'\n' if quote == b'"' => return at,
            b if b == quote => return at + 1,
            _ => at += 1,
        }
    }
    bytes.len()
}

/// The length of a character literal like `'('` or `'\''` at the start of
/// `rest`. A lone `'`, like a Rust lifetime, isn't one.
fn char_literal(rest: &[u8]) -> Option<usize> {
    match rest {
        [b'\'', b'\\', _, b'\'', ..] => Some(4),
        [b'\'', c, b'\'', ..] if *c != b'\\' => Some(3),
        _ => None,
    }
}

fn is_bracket(byte: u8) -> bool {
    matches!(byte, b'(' | b')' | b'[' | b']' | b'{' | b'}')
}

fn is_opening(byte: u8) -> bool {
    matches!(byte, b'(' | b'[' | b'{')
}

fn closing_for(open: u8) -> u8 {
    match open {
        b'(' => b')',
        b'[' => b']',
        _ => b'}',
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(text: &str, cursor: &str) -> Option<(usize, usize)> {
        // `cursor` marks where the cursor is, e.g. "foo(|".
        let at = cursor.find('|').unwrap();
        matching(text, at, Some("//"))
    }

    #[test]
    fn the_bracket_before_the_cursor_wins() {
        let text = "f(a)(b)";
        assert_eq!(pair(text, "f(a)|"), Some((1, 3)));
        assert_eq!(pair(text, "f(|"), Some((1, 3)));
        assert_eq!(pair(text, "|"), None);
        assert_eq!(pair(text, "f|"), Some((1, 3)));
    }

    #[test]
    fn nested_brackets_pair_up() {
        let text = "{ a[(1)] }";
        assert_eq!(pair(text, "{|"), Some((0, 9)));
        assert_eq!(pair(text, "{ a[|"), Some((3, 7)));
        assert_eq!(pair(text, "{ a[(|"), Some((4, 6)));
        assert_eq!(matching(text, 10, None), Some((0, 9)));
    }

    #[test]
    fn strings_and_comments_dont_count() {
        let text = "f(\")\", ')', x) // )\n/* ( */";
        assert_eq!(pair(text, "f(|"), Some((1, 13)));
        // A lifetime isn't a character literal.
        let text = "fn f<'a>(x: &'a str) {}";
        assert_eq!(matching(text, 9, None), Some((8, 19)));
    }

    #[test]
    fn hash_comments_only_where_the_language_has_them() {
        let text = "x = (1) # )";
        assert_eq!(matching(text, 5, Some("#")), Some((4, 6)));
        let text = "#[derive(Debug)]";
        assert_eq!(matching(text, 2, Some("//")), Some((1, 15)));
    }

    #[test]
    fn mismatched_or_unclosed_brackets_have_no_pair() {
        assert_eq!(matching("(]", 1, None), None);
        assert_eq!(matching("(a", 1, None), None);
    }
}
