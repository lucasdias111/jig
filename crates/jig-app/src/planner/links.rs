//! Links from a task to the code: `[`parse_reply`](src/response.rs#L42)`.
//! The line is a hint; the name is what's looked for, nearest that line,
//! so a link still lands on its function after the file has changed.

use std::ops::Range;

use regex::Regex;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeLink {
    /// Relative to the project.
    pub path: String,
    /// From 1, as written in the link.
    pub line: Option<usize>,
    /// The name it points at: the link's text, or `#name` in the link.
    pub symbol: Option<String>,
}

/// What clicking `url` in a task should do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Code(CodeLink),
    Web(String),
}

/// Read `url`, a link in `markdown`, whose text names the symbol.
pub fn target(url: &str, markdown: &str) -> Option<Target> {
    if url.contains("://") || url.starts_with("mailto:") {
        return Some(Target::Web(url.to_string()));
    }
    let (path, fragment) = url.split_once('#').unwrap_or((url, ""));
    let path = path.trim_start_matches("./").replace("%20", " ");
    if path.is_empty() {
        return None;
    }
    let line = line_of(fragment);
    let symbol = match line {
        None if !fragment.is_empty() => symbol_in(fragment),
        // A link named after its file points at the file, not a name in it.
        _ => text_of(url, markdown)
            .filter(|text| {
                let text = text.trim().trim_matches('`');
                !(text.contains(['.', '/']) && path.ends_with(text))
            })
            .and_then(|text| symbol_in(&text)),
    };
    Some(Target::Code(CodeLink { path, line, symbol }))
}

/// `L42`, `L42C5` or `L42-L50`.
fn line_of(fragment: &str) -> Option<usize> {
    let digits: String = fragment
        .strip_prefix('L')?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok().filter(|line| *line > 0)
}

/// The text of the first link to `url` in `markdown`.
fn text_of(url: &str, markdown: &str) -> Option<String> {
    let link = Regex::new(r"\[([^\]]+)\]\(([^)\s]+)\)").ok()?;
    link.captures_iter(markdown)
        .find(|captures| &captures[2] == url)
        .map(|captures| captures[1].to_string())
}

/// The name in a link's text: `` `Workspace::open_at()` `` → `open_at`.
/// `None` for text that isn't a name, like "the parser".
fn symbol_in(text: &str) -> Option<String> {
    let text = text.trim().trim_matches('`').trim();
    let text = text.strip_suffix("()").unwrap_or(text);
    let last = text
        .rsplit(['.', ':', '#', '/'])
        .find(|part| !part.is_empty())?;
    let name = last
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
    (name && !last.is_empty()).then(|| last.to_string())
}

/// Words that start a declaration in most languages, to pick a symbol's
/// declaration over its uses when the link gives no line.
const DECLARATIONS: [&str; 17] = [
    "fn",
    "struct",
    "enum",
    "trait",
    "type",
    "impl",
    "mod",
    "class",
    "interface",
    "def",
    "func",
    "function",
    "const",
    "let",
    "var",
    "static",
    "val",
];

/// Where `link` points in `text`: its symbol, the use nearest its line (or
/// its declaration, without a line); else the start of the line. `None`
/// when there's nothing to go to but the file.
pub fn locate(text: &str, link: &CodeLink) -> Option<Range<usize>> {
    let line_start = |line: usize| -> Option<usize> {
        if line <= 1 {
            return Some(0);
        }
        text.match_indices('\n').nth(line - 2).map(|(at, _)| at + 1)
    };
    if let Some(symbol) = &link.symbol {
        let uses = whole_words(text, symbol);
        let best = match link.line.and_then(line_start) {
            Some(near) => uses.iter().min_by_key(|at| at.abs_diff(near)).copied(),
            None => uses
                .iter()
                .find(|at| declares(text, **at))
                .or(uses.first())
                .copied(),
        };
        if let Some(at) = best {
            return Some(at..at + symbol.len());
        }
    }
    let start = line_start(link.line?)?;
    let indent = text[start..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .count();
    Some(start + indent..start + indent)
}

fn whole_words(text: &str, word: &str) -> Vec<usize> {
    let part = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    text.match_indices(word)
        .map(|(at, _)| at)
        .filter(|at| {
            let before = text[..*at].chars().next_back();
            let after = text[at + word.len()..].chars().next();
            !before.is_some_and(part) && !after.is_some_and(part)
        })
        .collect()
}

/// Whether the word at `at` follows a declaration keyword on its line.
fn declares(text: &str, at: usize) -> bool {
    let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);
    text[line_start..at]
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .any(|word| DECLARATIONS.contains(&word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(url: &str, markdown: &str) -> CodeLink {
        match target(url, markdown) {
            Some(Target::Code(link)) => link,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn reads_links_to_code() {
        let markdown = "Uses [`Workspace::open_at()`](src/w.rs#L42) and [the parser](./src/p.rs).";
        assert_eq!(
            code("src/w.rs#L42", markdown),
            CodeLink {
                path: "src/w.rs".into(),
                line: Some(42),
                symbol: Some("open_at".into())
            }
        );
        let parser = code("./src/p.rs", markdown);
        assert_eq!((parser.path.as_str(), parser.symbol), ("src/p.rs", None));
        assert_eq!(code("src/a.rs#run", "").symbol.as_deref(), Some("run"));
        assert_eq!(code("src/a.rs#L3-L9", "").line, Some(3));
        assert_eq!(
            code("src/main.rs", "[`main.rs`](src/main.rs)").symbol,
            None,
            "named after the file"
        );
        assert_eq!(
            target("https://x.dev/a#b", ""),
            Some(Target::Web("https://x.dev/a#b".into()))
        );
    }

    #[test]
    fn finds_the_symbol_nearest_its_line() {
        let text = "use a::run;\n\nfn run() {}\n\nfn main() {\n    run();\n}\n";
        let link = |line| CodeLink {
            path: "a.rs".into(),
            line,
            symbol: Some("run".into()),
        };
        let declaration = text.find("fn run").unwrap() + 3;
        assert_eq!(
            locate(text, &link(Some(3))),
            Some(declaration..declaration + 3)
        );
        let call = text.find("run();").unwrap();
        assert_eq!(
            locate(text, &link(Some(7))),
            Some(call..call + 3),
            "moved down"
        );
        assert_eq!(
            locate(text, &link(None)),
            Some(declaration..declaration + 3),
            "no line: the declaration"
        );
    }

    #[test]
    fn falls_back_to_the_line() {
        let text = "a\n    b\nc\n";
        let link = CodeLink {
            path: "a".into(),
            line: Some(2),
            symbol: Some("gone".into()),
        };
        assert_eq!(locate(text, &link), Some(6..6));
        let file = CodeLink {
            line: None,
            symbol: None,
            ..link
        };
        assert_eq!(locate(text, &file), None);
        assert!(whole_words("running run", "run") == vec![8]);
    }
}
