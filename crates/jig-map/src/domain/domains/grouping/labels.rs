//! Naming a group by its folder or its functions' commonest words.

use super::*;

pub(super) fn folder_label(part: &Part) -> String {
    let dir = place_of(&part.dir);
    let dir = if dir.is_empty() { "the project" } else { &dir };
    if part.merged.is_empty() {
        dir.to_string()
    } else {
        format!("{dir} and more")
    }
}

/// The three words `functions`' names use most, past the ones every
/// project uses; without functions, the files' names.
pub(super) fn top_words(project: &Project, functions: &[usize], files: &[PathBuf]) -> String {
    const COMMON: &[&str] = &[
        "get", "set", "new", "from", "with", "into", "make", "test", "tests", "show", "open",
        "this", "self", "default", "the", "for", "and", "all", "has", "can", "add", "fmt",
        "handle", "create", "update", "render", "value", "should", "when", "use", "data", "item",
        "list", "string", "type", "init", "main", "run", "read", "write", "build", "load", "save",
        "find", "check", "apply", "event", "context", "props", "state", "map", "key", "next",
        "its", "not", "are", "one", "out", "via", "but", "mod", "lib", "index", "impl", "drop",
        "clone", "eq", "from_str", "to", "is", "on", "of", "in", "at", "by", "or", "an", "it",
    ];
    let mut counts: HashMap<String, usize> = HashMap::new();
    let count = |counts: &mut HashMap<String, usize>, name: &str| {
        for word in words(name) {
            if word.len() >= 3 && !COMMON.contains(&word.as_str()) {
                *counts.entry(word).or_default() += 1;
            }
        }
    };
    for &id in functions {
        count(&mut counts, project.functions[id].short());
    }
    if counts.is_empty() {
        for file in files {
            if let Some(stem) = file.file_stem() {
                count(&mut counts, &stem.to_string_lossy());
            }
        }
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let words: Vec<String> = ranked.into_iter().take(3).map(|(w, _)| w).collect();
    if words.is_empty() {
        "unnamed".into()
    } else {
        words.join(" · ")
    }
}

/// A name's words, lower case: `bookRoom`, `book_room` and `BookRoom` are
/// all "book" and "room".
pub(super) fn words(name: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut last_lower = false;
    for c in name.chars() {
        let boundary = !c.is_alphanumeric() || (c.is_uppercase() && last_lower);
        if boundary && !word.is_empty() {
            words.push(std::mem::take(&mut word));
        }
        if c.is_alphanumeric() {
            word.extend(c.to_lowercase());
        }
        last_lower = c.is_lowercase() || c.is_ascii_digit();
    }
    if !word.is_empty() {
        words.push(word);
    }
    words
}
