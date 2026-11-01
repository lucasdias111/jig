//! Snippets: text with places to fill in, such as
//! `for ${1:item} in ${2:items} {\n\t$0\n}`.
//!
//! They come from language servers, in the LSP snippet syntax, and from Jig,
//! which reads `assets/default-snippets.toml` and
//! `~/.config/jig/snippets.toml` (`store::cache`; a user snippet replaces a
//! built-in with the same name). Prefix snippets are typed by name (`for`). Postfix ones go
//! after an expression (`users.var`) and wrap it: `$EXPR` is the expression,
//! `$NAME` a name guessed from it.

use std::collections::BTreeMap;
use std::ops::Range;

use serde::Deserialize;

mod postfix;

pub use postfix::{name_for, receiver};

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

/// Jig's snippets for `language` from the built-in ones and the user's,
/// the user's replacing built-ins of the same name.
pub fn merge(builtin: &[Snippet], user: Vec<Snippet>, language: &str) -> Vec<Snippet> {
    let mut snippets: Vec<Snippet> = builtin
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

/// The snippets in a snippets file's text.
pub fn from_toml(text: &str) -> Result<Vec<Snippet>, toml::de::Error> {
    toml::from_str::<SnippetFile>(text).map(|file| file.snippets)
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

#[cfg(test)]
mod tests;
