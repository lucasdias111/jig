//! Linking to code from a task with `@`: what's being typed, what it
//! names in the project, and the link each pick puts in,
//! ``[`name`](path#L12)``.

use super::symbols::{ProjectIndex, Symbol};

/// How many names the `@` list shows.
pub const MENTION_ROWS: usize = 8;

/// Something `@` can link to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Mentioned {
    Symbol(Symbol),
    /// A file, relative to the project.
    File(String),
}

impl Mentioned {
    /// The Markdown link to it.
    pub fn link(&self) -> String {
        match self {
            Mentioned::Symbol(symbol) => {
                format!("[`{}`]({}#L{})", symbol.name, symbol.path, symbol.line)
            }
            Mentioned::File(path) => {
                let name = path.rsplit('/').next().unwrap_or(path);
                format!("[`{name}`]({path})")
            }
        }
    }
}

/// The `@` and the name typed after it, when the cursor is right after
/// one: `See @par|` gives the `@`'s offset and `par`. An `@` inside a word,
/// like an email address's, isn't one.
pub fn mention_at(text: &str, cursor: usize) -> Option<(usize, &str)> {
    let before = text.get(..cursor)?;
    let at = before.rfind('@')?;
    let query = &before[at + 1..];
    let name = query
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '$' | ':'));
    let starts = before[..at]
        .chars()
        .next_back()
        .is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '['));
    (name && starts).then_some((at, query))
}

/// What `query` names in `index`: declarations and files, best first.
/// With nothing typed, the names in `current`, the open file.
pub fn matches(index: &ProjectIndex, current: Option<&str>, query: &str) -> Vec<Mentioned> {
    if query.is_empty() {
        return index
            .symbols
            .iter()
            .filter(|symbol| Some(symbol.path.as_str()) == current)
            .take(MENTION_ROWS)
            .map(|symbol| Mentioned::Symbol(symbol.clone()))
            .collect();
    }
    let lower = query.to_lowercase();
    let names = jig_commands::fuzzy::match_paths(
        index.symbols.iter().map(|symbol| symbol.name.as_str()),
        query,
        MENTION_ROWS * 4,
    );
    let files = jig_commands::fuzzy::match_paths(
        index.files.iter().map(String::as_str),
        query,
        MENTION_ROWS,
    );
    let mut scored: Vec<(i32, Mentioned)> = names
        .into_iter()
        .map(|found| {
            let symbol = &index.symbols[found.index];
            let name = symbol.name.to_lowercase();
            // A name that is, or starts with, what's typed beats one
            // that merely contains its letters.
            let bonus = if name == lower {
                3000
            } else if name.starts_with(&lower) {
                1500
            } else {
                0
            };
            (found.score + bonus, Mentioned::Symbol(symbol.clone()))
        })
        .chain(files.into_iter().map(|found| {
            (
                found.score,
                Mentioned::File(index.files[found.index].clone()),
            )
        }))
        .collect();
    scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    scored
        .into_iter()
        .take(MENTION_ROWS)
        .map(|(_, item)| item)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_mention_before_the_cursor() {
        assert_eq!(mention_at("See @par", 8), Some((4, "par")));
        assert_eq!(mention_at("@", 1), Some((0, "")));
        assert_eq!(mention_at("(@Cache::get", 12), Some((1, "Cache::get")));
        assert_eq!(mention_at("See @par and", 12), None, "past a space");
        assert_eq!(mention_at("me@example.com", 14), None, "inside a word");
        assert_eq!(
            mention_at("See @par", 6),
            Some((4, "p")),
            "up to the cursor"
        );
    }

    #[test]
    fn links_name_their_place() {
        let symbol = Mentioned::Symbol(Symbol {
            name: "run".into(),
            kind: "fn".into(),
            path: "src/a.rs".into(),
            line: 3,
        });
        assert_eq!(symbol.link(), "[`run`](src/a.rs#L3)");
        assert_eq!(
            Mentioned::File("src/a.rs".into()).link(),
            "[`a.rs`](src/a.rs)"
        );
    }
}
