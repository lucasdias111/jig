//! Snippets: text with places to fill in, such as
//! `for ${1:item} in ${2:items} {\n\t$0\n}`.
//!
//! They come from language servers, in the LSP snippet syntax, and from Jig,
//! which reads `assets/default-snippets.toml` and
//! `~/.config/jig/snippets.toml` (a user snippet replaces a built-in with the
//! same name). Prefix snippets are typed by name (`for`). Postfix ones go
//! after an expression (`users.var`) and wrap it: `$EXPR` is the expression,
//! `$NAME` a name guessed from it.

use std::collections::BTreeMap;
use std::ops::Range;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use serde::Deserialize;

/// A snippet with its places filled by their defaults, and where those
/// places are, in the order Tab visits them. The last is where the cursor
/// ends up. Empty when there's nowhere to go but the end.
#[derive(Clone, Debug, PartialEq)]
pub struct Expansion {
    pub text: String,
    pub stops: Vec<Range<usize>>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Snippet {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Jig's names for the languages it's for; `*` for all.
    pub languages: Vec<String>,
    /// Typed after an expression and a dot, rather than on its own.
    #[serde(default)]
    pub postfix: bool,
    pub body: String,
}

#[derive(Default, Deserialize)]
struct SnippetFile {
    #[serde(default)]
    snippets: Vec<Snippet>,
}

impl Snippet {
    fn is_for(&self, language: &str) -> bool {
        self.languages.iter().any(|l| l == language || l == "*")
    }
}

/// Jig's snippets for `language`, the user's replacing built-ins of the same
/// name.
pub fn for_language(language: &str) -> Vec<Snippet> {
    let user = user_snippets();
    let mut snippets: Vec<Snippet> = builtin()
        .iter()
        .filter(|s| s.is_for(language))
        .filter(|s| {
            !user
                .iter()
                .any(|own| own.is_for(language) && own.name == s.name && own.postfix == s.postfix)
        })
        .cloned()
        .collect();
    snippets.extend(user.into_iter().filter(|s| s.is_for(language)));
    snippets
}

fn builtin() -> &'static [Snippet] {
    static BUILTIN: OnceLock<Vec<Snippet>> = OnceLock::new();
    BUILTIN.get_or_init(|| {
        toml::from_str::<SnippetFile>(include_str!("../../../assets/default-snippets.toml"))
            .expect("the built-in snippets parse")
            .snippets
    })
}

/// `~/.config/jig/snippets.toml`, honouring `XDG_CONFIG_HOME`.
pub fn user_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("jig").join("snippets.toml"))
}

/// The user's snippets, read again only when the file changes, as this is
/// asked on every keystroke.
fn user_snippets() -> Vec<Snippet> {
    static CACHE: Mutex<Option<(SystemTime, Vec<Snippet>)>> = Mutex::new(None);
    if cfg!(test) {
        return Vec::new();
    }
    let Some(path) = user_path() else {
        return Vec::new();
    };
    let Ok(modified) = std::fs::metadata(&path).and_then(|m| m.modified()) else {
        return Vec::new();
    };
    let mut cache = CACHE.lock().unwrap();
    if let Some((when, snippets)) = cache.as_ref()
        && *when == modified
    {
        return snippets.clone();
    }
    let snippets = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| match toml::from_str::<SnippetFile>(&text) {
            Ok(file) => Some(file.snippets),
            Err(error) => {
                eprintln!("jig: {}: {error}", path.display());
                None
            }
        })
        .unwrap_or_default();
    *cache = Some((modified, snippets.clone()));
    snippets
}

/// `body` ready to insert on a line indented by `line_indent`: each new
/// line indented to match, and each tab one `unit` of indentation.
pub fn indent(body: &str, line_indent: &str, unit: &str) -> String {
    body.replace('\t', unit)
        .replace('\n', &format!("\n{line_indent}"))
}

/// A postfix snippet's body around `expr`, ready to [`parse`].
pub fn wrap(body: &str, expr: &str, language: &str) -> String {
    let escape = |s: &str| {
        s.replace('\\', "\\\\")
            .replace('$', "\\$")
            .replace('}', "\\}")
    };
    body.replace("$NAME", &escape(&name_for(expr, language)))
        .replace("$EXPR", &escape(expr))
}

/// Expand an LSP snippet: `push(${1:value})$0` is `push(value)`, with
/// `value` the first place and the end the last.
pub fn parse(snippet: &str) -> Expansion {
    let mut parser = Parser {
        chars: snippet.chars().collect(),
        at: 0,
        out: String::new(),
        stops: BTreeMap::new(),
    };
    parser.body(false);
    let end = parser.out.len();
    let mut stops: Vec<Range<usize>> = parser
        .stops
        .iter()
        .filter(|(index, _)| **index != 0)
        .map(|(_, range)| range.clone())
        .collect();
    stops.push(parser.stops.get(&0).cloned().unwrap_or(end..end));
    if stops.len() == 1 && stops[0] == (end..end) {
        stops.clear();
    }
    Expansion {
        text: parser.out,
        stops,
    }
}

struct Parser {
    chars: Vec<char>,
    at: usize,
    out: String,
    /// Each place's first appearance; later ones repeat its text.
    stops: BTreeMap<u32, Range<usize>>,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += 1;
        Some(c)
    }

    /// Up to the end, or inside a placeholder up to its closing `}`.
    fn body(&mut self, nested: bool) {
        while let Some(c) = self.next() {
            match c {
                '\\' => match self.peek() {
                    Some(escaped @ ('$' | '}' | '\\')) => {
                        self.at += 1;
                        self.out.push(escaped);
                    }
                    _ => self.out.push('\\'),
                },
                '}' if nested => return,
                '$' => self.dollar(),
                c => self.out.push(c),
            }
        }
    }

    fn number(&mut self) -> Option<u32> {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.at += 1;
        }
        self.chars[start..self.at]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }

    fn variable(&mut self) {
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        {
            self.at += 1;
        }
    }

    fn dollar(&mut self) {
        match self.peek() {
            Some(c) if c.is_ascii_digit() => {
                let index = self.number().unwrap_or(0);
                self.place(index, self.out.len());
            }
            Some('{') => {
                self.at += 1;
                if self.peek().is_some_and(|c| c.is_ascii_digit()) {
                    let index = self.number().unwrap_or(0);
                    let start = self.out.len();
                    match self.next() {
                        Some(':') => self.body(true),
                        Some('|') => {
                            let mut choices = String::new();
                            while let Some(c) = self.next() {
                                if c == '|' && self.peek() == Some('}') {
                                    self.at += 1;
                                    break;
                                }
                                choices.push(c);
                            }
                            self.out
                                .push_str(choices.split(',').next().unwrap_or_default());
                        }
                        _ => {}
                    }
                    self.place(index, start);
                } else {
                    // A variable, such as ${TM_SELECTED_TEXT:default}: Jig
                    // has none, so its default if it gives one.
                    self.variable();
                    if self.next() == Some(':') {
                        self.body(true);
                    }
                }
            }
            Some(c) if c.is_ascii_alphabetic() || c == '_' => self.variable(),
            _ => self.out.push('$'),
        }
    }

    /// Place `index` was written from `start` to here.
    fn place(&mut self, index: u32, start: usize) {
        match self.stops.get(&index) {
            Some(first) if start == self.out.len() => {
                // `$1` again with nothing of its own: repeat the first.
                let text = self.out[first.clone()].to_string();
                self.out.push_str(&text);
            }
            Some(_) => {}
            None => {
                self.stops.insert(index, start..self.out.len());
            }
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// The text of each place in `snippet`, in order.
    fn places(snippet: &str) -> Vec<String> {
        let expansion = parse(snippet);
        expansion
            .stops
            .iter()
            .map(|r| expansion.text[r.clone()].to_string())
            .collect()
    }

    #[test]
    fn places_in_order_then_the_end() {
        let expansion = parse("for ${1:item} in ${2:items} {\n\t$0\n}");
        assert_eq!(expansion.text, "for item in items {\n\t\n}");
        assert_eq!(expansion.stops, [4..8, 12..17, 21..21]);
    }

    #[test]
    fn without_a_final_place_the_end_is_last() {
        let expansion = parse("push(${1:value})");
        assert_eq!(expansion.stops, [5..10, 11..11]);
    }

    #[test]
    fn plain_text_has_no_places() {
        assert!(parse("push").stops.is_empty());
        assert!(parse("push()$0").stops.is_empty());
        assert_eq!(parse("println!(\"$0\")").stops.first(), Some(&(10..10)));
    }

    #[test]
    fn nested_choices_variables_and_escapes() {
        assert_eq!(parse("${1:Vec<${2:T}>}").text, "Vec<T>");
        assert_eq!(places("${1:Vec<${2:T}>}"), ["Vec<T>", "T", ""]);
        assert_eq!(parse("${1|one,two|}").text, "one");
        assert_eq!(parse("$TM_SELECTED_TEXT${TM_FILENAME:x}").text, "x");
        assert_eq!(parse("cost \\$5 \\} $").text, "cost $5 } $");
    }

    #[test]
    fn a_place_repeated_repeats_its_text() {
        let snippet = "for (let ${1:i} = 0; $1 < n; $1++) {$0}";
        assert_eq!(parse(snippet).text, "for (let i = 0; i < n; i++) {}");
        assert_eq!(places(snippet), ["i", ""]);
    }

    #[test]
    fn indented_to_the_line() {
        assert_eq!(
            indent("if x {\n\t$0\n}", "    ", "    "),
            "if x {\n        $0\n    }"
        );
    }

    #[test]
    fn receivers() {
        fn at(text: &str) -> Option<&str> {
            let dot = text.rfind('.').unwrap();
            receiver(text, dot).map(|r| &text[r])
        }
        assert_eq!(at("    users.v"), Some("users"));
        assert_eq!(at("let x = self.items.len()."), Some("self.items.len()"));
        assert_eq!(at("foo(bar(1), [2]).x"), Some("foo(bar(1), [2])"));
        assert_eq!(at("x = Vec::new()."), Some("Vec::new()"));
        assert_eq!(at("vec![1, 2]."), Some("vec![1, 2]"));
        assert_eq!(at("\"a.b\"."), Some("\"a.b\""));
        assert_eq!(at("a?.b?.c."), Some("a?.b?.c"));
        assert_eq!(at("load()?."), Some("load()?"));
        assert_eq!(at("  ."), None);
    }

    #[test]
    fn names() {
        assert_eq!(name_for("users", "rust"), "users");
        assert_eq!(name_for("get_user(id)", "rust"), "user");
        assert_eq!(name_for("getUser(id)", "typescript"), "user");
        assert_eq!(name_for("HttpClient::new()", "rust"), "http_client");
        assert_eq!(name_for("HttpClient::new()", "go"), "httpClient");
        assert_eq!(name_for("self.items.clone()", "rust"), "items");
        assert_eq!(name_for("\"hello\"", "python"), "value");
        assert_eq!(name_for("fetchURLList()", "javascript"), "urlList");
        assert_eq!(name_for("42", "rust"), "value");
    }

    #[test]
    fn postfix_bodies_get_the_expression_and_a_name() {
        assert_eq!(
            parse(&wrap(
                "let ${1:$NAME} = $EXPR;",
                "get_user(\"$x}\")",
                "rust"
            ))
            .text,
            "let user = get_user(\"$x}\");"
        );
    }

    #[test]
    fn every_built_in_snippet_parses_and_has_a_language() {
        let known = [
            "rust",
            "typescript",
            "tsx",
            "javascript",
            "java",
            "python",
            "go",
        ];
        for snippet in builtin() {
            assert!(
                snippet
                    .languages
                    .iter()
                    .all(|l| known.contains(&l.as_str())),
                "{}",
                snippet.name
            );
            let body = if snippet.postfix {
                wrap(&snippet.body, "items", "rust")
            } else {
                snippet.body.clone()
            };
            assert!(!parse(&body).text.contains('$'), "{}", snippet.name);
        }
        for language in known {
            let names: Vec<String> = for_language(language)
                .iter()
                .map(|s| format!("{}{}", if s.postfix { "." } else { "" }, s.name))
                .collect();
            for wanted in ["for", ".var", ".for", ".if"] {
                assert!(
                    names.iter().any(|n| n == wanted),
                    "{language} lacks {wanted}"
                );
            }
        }
    }
}
