//! Reading the source as tokens: the files of a crate, and the `crate::` and `super::` paths each one names.

use super::*;

/// Every source file under `src`, with its module path (`main.rs` or
/// `lib.rs` is the crate root, `a/mod.rs` is `a`). Test modules are left out.
pub(super) fn source_files(source: &Path) -> Vec<(PathBuf, Vec<String>)> {
    let mut files = Vec::new();
    let mut folders = vec![source.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let relative = path.strip_prefix(source).unwrap().with_extension("");
            let mut here: Vec<String> = relative
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            if matches!(
                here.last().map(String::as_str),
                Some("main" | "lib" | "mod")
            ) {
                here.pop();
            }
            if here.iter().any(|segment| segment == "tests") {
                continue;
            }
            files.push((path, here));
        }
    }
    files.sort();
    files
}

/// What a file's tokens say: the inline modules it declares and every path
/// it names from the crate's root, resolved to absolute segments.
#[derive(Default)]
pub(super) struct Scan {
    /// The crate root's own file, where `map::X` means `crate::map::X`.
    pub(super) root: bool,
    /// The crates of capabilities that have their own, as code names them.
    pub(super) crates: Vec<String>,
    pub(super) modules: Vec<Vec<String>>,
    pub(super) paths: Vec<Vec<String>>,
}

impl Scan {
    pub(super) fn tokens(&mut self, tokens: TokenStream, here: &[String]) {
        let tokens: Vec<TokenTree> = tokens.into_iter().collect();
        let mut at = 0;
        while at < tokens.len() {
            match &tokens[at] {
                TokenTree::Punct(p) if p.as_char() == '#' && is_cfg_test(tokens.get(at + 1)) => {
                    at = after_item(&tokens, at + 2);
                    continue;
                }
                TokenTree::Ident(word) if word == "mod" => {
                    if let (Some(TokenTree::Ident(name)), Some(TokenTree::Group(body))) =
                        (tokens.get(at + 1), tokens.get(at + 2))
                        && body.delimiter() == Delimiter::Brace
                    {
                        let mut inner = here.to_vec();
                        inner.push(name.to_string());
                        self.modules.push(inner.clone());
                        let root = std::mem::replace(&mut self.root, false);
                        self.tokens(body.stream(), &inner);
                        self.root = root;
                        at += 3;
                        continue;
                    }
                }
                TokenTree::Ident(word) if !follows_separator(&tokens, at) => {
                    let start = match word.to_string().as_str() {
                        "crate" => Some(Vec::new()),
                        "self" => Some(here.to_vec()),
                        "super" => Some(here[..here.len().saturating_sub(1)].to_vec()),
                        name if (self.root || self.crates.iter().any(|c| c == name))
                            && is_separator(&tokens, at + 1) =>
                        {
                            Some(vec![name.to_string()])
                        }
                        _ => None,
                    };
                    if let Some(start) = start
                        && is_separator(&tokens, at + 1)
                    {
                        at = rest_of_path(&tokens, at + 1, start, &mut self.paths);
                        continue;
                    }
                }
                TokenTree::Group(group) => self.tokens(group.stream(), here),
                _ => {}
            }
            at += 1;
        }
    }
}

/// Reads a path on from the `::` at `at`, its start already resolved, adding
/// every full path it names (`a::{b, c::D}` names two) and returning where
/// it ends.
pub(super) fn rest_of_path(
    tokens: &[TokenTree],
    mut at: usize,
    mut path: Vec<String>,
    out: &mut Vec<Vec<String>>,
) -> usize {
    while is_separator(tokens, at) {
        match tokens.get(at + 2) {
            Some(TokenTree::Ident(segment)) => {
                match segment.to_string().as_str() {
                    "super" => {
                        path.pop();
                    }
                    "self" => {}
                    segment => path.push(segment.to_string()),
                }
                at += 3;
            }
            Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => {
                for part in split_at_commas(group.stream()) {
                    let mut inner = path.clone();
                    match part.first() {
                        Some(TokenTree::Ident(first)) if first != "self" => {
                            inner.push(first.to_string());
                            rest_of_path(&part, 1, inner, out);
                        }
                        _ => out.push(inner),
                    }
                }
                return at + 3;
            }
            _ => break,
        }
    }
    out.push(path);
    at
}

fn split_at_commas(stream: TokenStream) -> Vec<Vec<TokenTree>> {
    let mut parts = vec![Vec::new()];
    for token in stream {
        match &token {
            TokenTree::Punct(p) if p.as_char() == ',' => parts.push(Vec::new()),
            _ => parts.last_mut().unwrap().push(token),
        }
    }
    parts.retain(|part| !part.is_empty());
    parts
}

pub(super) fn is_separator(tokens: &[TokenTree], at: usize) -> bool {
    matches!(
        (tokens.get(at), tokens.get(at + 1)),
        (Some(TokenTree::Punct(a)), Some(TokenTree::Punct(b)))
            if a.as_char() == ':' && a.spacing() == Spacing::Joint && b.as_char() == ':'
    )
}

pub(super) fn follows_separator(tokens: &[TokenTree], at: usize) -> bool {
    at >= 2 && is_separator(tokens, at - 2)
}

pub(super) fn is_cfg_test(token: Option<&TokenTree>) -> bool {
    let Some(TokenTree::Group(group)) = token else {
        return false;
    };
    let text: String = group
        .stream()
        .to_string()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    group.delimiter() == Delimiter::Bracket && text == "cfg(test)"
}

/// Where the item starting at `at` ends: after its first `;` or `{ … }`,
/// past any further attributes.
pub(super) fn after_item(tokens: &[TokenTree], mut at: usize) -> usize {
    while at < tokens.len() {
        match &tokens[at] {
            TokenTree::Punct(p) if p.as_char() == ';' => return at + 1,
            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => return at + 1,
            _ => at += 1,
        }
    }
    at
}
