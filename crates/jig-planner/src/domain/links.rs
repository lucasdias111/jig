//! Links from a task to the code: `[`parse_reply`](src/response.rs#L42)`.
//! The line is a hint; the name is what's looked for, nearest that line,
//! so a link still lands on its function after the file has changed.

use std::ops::Range;
use std::sync::LazyLock;

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

/// The first link to code in `markdown`, e.g. a todo's.
pub fn first_code_link(markdown: &str) -> Option<CodeLink> {
    let link = Regex::new(r"\[[^\]]+\]\(([^)\s]+)\)").ok()?;
    link.captures_iter(markdown)
        .find_map(|captures| match target(&captures[1], markdown) {
            Some(Target::Code(link)) => Some(link),
            _ => None,
        })
}

/// A file named in prose: `src/a.rs`, `a.rs:42`, `/abs/a.rs:42:5`.
static BARE_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:^|[\s(\[{<'])((?:\.{1,2}/|/)?[\w\-.]+(?:/[\w\-.]+)*\.[A-Za-z0-9]+)(?::(\d+)(?:[:\-–]\d+)?)?")
        .expect("a valid pattern")
});

/// A file in inline code: the whole span, with an optional `:42` or `#L42`.
static CODE_FILE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^((?:\.{1,2}/|/)?[\w\-.]+(?:/[\w\-.]+)*\.[A-Za-z0-9]+)(?:(?::|#L)(\d+)(?:[:\-–C]L?\d+)?)?$")
        .expect("a valid pattern")
});

/// A Markdown link, `[text](url)`, kept as it is.
static LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\[[^\]\n]*\]\([^)\s]*\)").expect("a valid pattern"));

/// `markdown` with the files it names left unlinked made links:
/// `` `src/a.rs:42` `` becomes ``[`src/a.rs:42`](src/a.rs#L42)``, and so
/// does a bare `src/a.rs:42`. `file` turns a path as written into the
/// project's path for it, or `None` when there's no such file, so only
/// real files are linked. Code blocks and links stay as they are.
pub fn link_files(markdown: &str, file: impl Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut fenced = false;
    for line in markdown.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
        }
        if fenced || trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            out.push_str(line);
        } else {
            link_files_in_line(line, &file, &mut out);
        }
    }
    out
}

fn link_files_in_line(line: &str, file: &impl Fn(&str) -> Option<String>, out: &mut String) {
    let mut plain = 0;
    let mut at = 0;
    while at < line.len() {
        let rest = &line[at..];
        if rest.starts_with('[')
            && let Some(link) = LINK.find(rest)
        {
            link_bare_files(&line[plain..at], file, out);
            out.push_str(link.as_str());
            at += link.end();
            plain = at;
        } else if rest.starts_with('`') {
            let ticks = rest.len() - rest.trim_start_matches('`').len();
            let fence = &rest[..ticks];
            let Some(close) = rest[ticks..].find(fence) else {
                break;
            };
            link_bare_files(&line[plain..at], file, out);
            let span = &rest[..ticks + close + ticks];
            let code = rest[ticks..ticks + close].trim();
            match CODE_FILE
                .captures(code)
                .and_then(|found| Some((file(&found[1])?, found.get(2))))
            {
                Some((path, line)) => {
                    out.push_str(&format!("[{span}]({path}{})", anchor(line)));
                }
                None => out.push_str(span),
            }
            at += span.len();
            plain = at;
        } else {
            at += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    link_bare_files(&line[plain..], file, out);
}

fn link_bare_files(text: &str, file: &impl Fn(&str) -> Option<String>, out: &mut String) {
    let mut from = 0;
    for found in BARE_FILE.captures_iter(text) {
        let whole = found.get(0).expect("the whole match");
        let path = found.get(1).expect("the path");
        // Part of a longer word or a URL, like `x.dev/a.rs` in `https://x.dev/a.rs`.
        let after = text[whole.end()..].chars().next();
        if after.is_some_and(|c| c.is_alphanumeric() || matches!(c, '_' | '/' | ':')) {
            continue;
        }
        let Some(project_path) = file(path.as_str()) else {
            continue;
        };
        out.push_str(&text[from..path.start()]);
        out.push_str(&format!(
            "[{}]({project_path}{})",
            &text[path.start()..whole.end()],
            anchor(found.get(2))
        ));
        from = whole.end();
    }
    out.push_str(&text[from..]);
}

/// `#L42` for a line, if there is one.
fn anchor(line: Option<regex::Match>) -> String {
    line.map(|line| format!("#L{}", line.as_str()))
        .unwrap_or_default()
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

    #[test]
    fn links_the_files_an_answer_names() {
        let file = |path: &str| {
            let path = path.strip_prefix("/p/").unwrap_or(path);
            ["src/a.rs", "Cargo.toml"]
                .contains(&path)
                .then(|| path.to_string())
        };
        let link = |markdown: &str| link_files(markdown, file);
        assert_eq!(
            link("See `src/a.rs:42` and src/a.rs:7:2."),
            "See [`src/a.rs:42`](src/a.rs#L42) and [src/a.rs:7:2](src/a.rs#L7)."
        );
        assert_eq!(
            link("In /p/src/a.rs, (Cargo.toml) and `src/a.rs#L3-L9`"),
            "In [/p/src/a.rs](src/a.rs), ([Cargo.toml](Cargo.toml)) and [`src/a.rs#L3-L9`](src/a.rs#L3)"
        );
        for kept in [
            "[`run`](src/a.rs#L1) and [src/a.rs](src/a.rs)",
            "`src/b.rs`, e.g. `fn a()` or `x`",
            "https://x.dev/src/a.rs and src/a.rsx",
            "```\nsrc/a.rs:1\n```\n",
            "an `unclosed src/a.rs",
        ] {
            let linked = link(kept);
            if kept.starts_with("an ") {
                assert_eq!(linked, "an `unclosed [src/a.rs](src/a.rs)");
            } else {
                assert_eq!(linked, kept);
            }
        }
    }

    #[test]
    fn finds_a_todos_first_link_to_code() {
        let todo = "Read [the docs](https://x.dev), then fix [`go`](src/b.rs#L1)";
        let link = first_code_link(todo).unwrap();
        assert_eq!(
            (link.path.as_str(), link.symbol.as_deref()),
            ("src/b.rs", Some("go"))
        );
        assert_eq!(first_code_link("No links"), None);
    }
}
