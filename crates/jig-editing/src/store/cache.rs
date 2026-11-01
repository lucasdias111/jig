//! Jig's snippets as read: the built-in ones, parsed once, and the user's,
//! read again only when their file changes.

use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::domain::snippets::{self, Snippet};

/// Jig's snippets for `language`, the user's replacing built-ins of the same
/// name.
pub fn for_language(language: &str) -> Vec<Snippet> {
    snippets::merge(builtin(), user_snippets(), language)
}

fn builtin() -> &'static [Snippet] {
    static BUILTIN: OnceLock<Vec<Snippet>> = OnceLock::new();
    BUILTIN.get_or_init(|| {
        snippets::from_toml(include_str!("../../../../assets/default-snippets.toml"))
            .expect("the built-in snippets parse")
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
    if cfg!(any(test, feature = "testing")) {
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
        .and_then(|text| match snippets::from_toml(&text) {
            Ok(snippets) => Some(snippets),
            Err(error) => {
                eprintln!("jig: {}: {error}", path.display());
                None
            }
        })
        .unwrap_or_default();
    *cache = Some((modified, snippets.clone()));
    snippets
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::snippets::{parse, wrap};

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
