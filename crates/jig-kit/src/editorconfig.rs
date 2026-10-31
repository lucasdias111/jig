//! The project's `.editorconfig` (<https://editorconfig.org>): what it says
//! about a file's indentation, trailing whitespace, final newline and line
//! endings, ahead of what Jig would guess or Settings says.
//!
//! The files are read from the file's folder upwards until one says
//! `root = true`; nearer files win over further ones, and later sections
//! over earlier ones. `charset` and `max_line_length` aren't used: Jig
//! reads and writes UTF-8 only and draws no ruler.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use gpui_kit::component::input::TabSize;
use regex::Regex;

const FILE_NAME: &str = ".editorconfig";

/// The properties that apply to one file, lowercased, `unset` ones gone.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EditorConfig {
    properties: HashMap<String, String>,
}

/// The line endings a file is saved with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineEnding {
    Lf,
    CrLf,
}

impl LineEnding {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lf => "\n",
            Self::CrLf => "\r\n",
        }
    }
}

impl EditorConfig {
    /// What the `.editorconfig` files above `path` say about it. Files that
    /// can't be read count as not there.
    pub fn for_file(path: &Path) -> Self {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let mut found = Vec::new();
        for dir in path.ancestors().skip(1) {
            let Ok(text) = fs::read_to_string(dir.join(FILE_NAME)) else {
                continue;
            };
            let file = parse(&text);
            let root = file.root;
            found.push((dir.to_path_buf(), file));
            if root {
                break;
            }
        }
        let mut config = Self::default();
        for (dir, file) in found.iter().rev() {
            let Ok(relative) = path.strip_prefix(dir) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            for section in &file.sections {
                if section.matches(&relative) {
                    for (key, value) in &section.properties {
                        if value == "unset" {
                            config.properties.remove(key);
                        } else {
                            config.properties.insert(key.clone(), value.clone());
                        }
                    }
                }
            }
        }
        config
    }

    /// `detected` with whatever the file sets of its style and size.
    pub fn tab_size(&self, detected: TabSize) -> TabSize {
        let hard_tabs = match self.get("indent_style") {
            Some("tab") => true,
            Some("space") => false,
            _ => detected.hard_tabs,
        };
        let number = |key| self.get(key).and_then(|v| v.parse::<usize>().ok());
        let tab_width = number("tab_width");
        // `indent_size = tab` means a tab's width; `tab_width` defaults to
        // the indent size.
        let indent_size = match self.get("indent_size") {
            Some("tab") => tab_width,
            _ => number("indent_size"),
        };
        let size = if hard_tabs {
            tab_width.or(indent_size)
        } else {
            indent_size.or(tab_width)
        };
        TabSize {
            tab_size: size.filter(|&size| size > 0).unwrap_or(detected.tab_size),
            hard_tabs,
        }
    }

    pub fn trim_trailing_whitespace(&self) -> Option<bool> {
        self.flag("trim_trailing_whitespace")
    }

    pub fn insert_final_newline(&self) -> Option<bool> {
        self.flag("insert_final_newline")
    }

    /// `cr` isn't followed: the editor only breaks lines at `\n`.
    pub fn end_of_line(&self) -> Option<LineEnding> {
        match self.get("end_of_line") {
            Some("lf") => Some(LineEnding::Lf),
            Some("crlf") => Some(LineEnding::CrLf),
            _ => None,
        }
    }

    fn get(&self, key: &str) -> Option<&str> {
        self.properties.get(key).map(String::as_str)
    }

    fn flag(&self, key: &str) -> Option<bool> {
        match self.get(key) {
            Some("true") => Some(true),
            Some("false") => Some(false),
            _ => None,
        }
    }
}

struct File {
    root: bool,
    sections: Vec<Section>,
}

struct Section {
    /// `None` for a glob that didn't make a pattern, which matches nothing.
    pattern: Option<Pattern>,
    properties: Vec<(String, String)>,
}

impl Section {
    fn matches(&self, path: &str) -> bool {
        self.pattern.as_ref().is_some_and(|p| p.matches(path))
    }
}

fn parse(text: &str) -> File {
    let mut file = File {
        root: false,
        sections: Vec::new(),
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            file.sections.push(Section {
                pattern: Pattern::new(&line[1..line.len() - 1]),
                properties: Vec::new(),
            });
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value.trim().to_lowercase();
        match file.sections.last_mut() {
            Some(section) => section.properties.push((key, value)),
            None if key == "root" => file.root = value == "true",
            None => {}
        }
    }
    file
}

/// A section's glob as a regex, with the number ranges (`{1..10}`) its
/// groups must fall in, which a regex can't say.
struct Pattern {
    regex: Regex,
    ranges: Vec<(i64, i64)>,
}

impl Pattern {
    /// A glob with a `/` is relative to the `.editorconfig`'s folder;
    /// without one it matches a file's name in any folder below.
    fn new(glob: &str) -> Option<Self> {
        let mut ranges = Vec::new();
        let chars: Vec<char> = glob.chars().collect();
        let body = translate(&chars, &mut ranges);
        let anchored = if glob.contains('/') {
            body.strip_prefix('/').unwrap_or(&body).to_string()
        } else {
            format!("(?:.*/)?{body}")
        };
        let regex = Regex::new(&format!("^{anchored}$")).ok()?;
        Some(Self { regex, ranges })
    }

    fn matches(&self, path: &str) -> bool {
        let Some(captures) = self.regex.captures(path) else {
            return false;
        };
        self.ranges.iter().enumerate().all(|(ix, &(low, high))| {
            captures
                .get(ix + 1)
                .and_then(|n| n.as_str().parse::<i64>().ok())
                .is_some_and(|n| (low..=high).contains(&n))
        })
    }
}

/// `glob` as a regex body: `*` within a folder, `**` across folders, `?`
/// one character, `[…]` and `[!…]` classes, `{a,b}` alternatives (nested
/// too), `{1..10}` numbers, `\` escaping. Anything unbalanced is literal.
fn translate(glob: &[char], ranges: &mut Vec<(i64, i64)>) -> String {
    let mut out = String::new();
    let mut ix = 0;
    while ix < glob.len() {
        let c = glob[ix];
        match c {
            '\\' if ix + 1 < glob.len() => {
                ix += 1;
                out.push_str(&regex::escape(&glob[ix].to_string()));
            }
            '*' if glob.get(ix + 1) == Some(&'*') => {
                ix += 1;
                out.push_str(".*");
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            '[' => match class(&glob[ix..]) {
                Some((class, len)) => {
                    out.push_str(&class);
                    ix += len - 1;
                }
                None => out.push_str(r"\["),
            },
            '{' => match closing_brace(&glob[ix..]) {
                Some(len) => {
                    out.push_str(&braces(&glob[ix + 1..ix + len - 1], ranges));
                    ix += len - 1;
                }
                None => out.push_str(r"\{"),
            },
            _ => out.push_str(&regex::escape(&c.to_string())),
        }
        ix += 1;
    }
    out
}

/// The class `[…]` at the start of `glob` and how many characters it
/// takes, or `None` if it isn't closed (or crosses a `/`).
fn class(glob: &[char]) -> Option<(String, usize)> {
    let mut ix = 1;
    let mut out = String::from("[");
    if glob.get(ix) == Some(&'!') {
        out.push('^');
        ix += 1;
    }
    let start = ix;
    while ix < glob.len() {
        match glob[ix] {
            ']' if ix > start => return Some((out + "]", ix + 1)),
            '/' => return None,
            '\\' if ix + 1 < glob.len() => {
                ix += 1;
                out.push('\\');
                out.push(glob[ix]);
            }
            c @ ('[' | ']' | '^' | '&' | '~') => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
        ix += 1;
    }
    None
}

/// How many characters the `{…}` at the start of `glob` takes, nested
/// braces included, or `None` if it isn't closed.
fn closing_brace(glob: &[char]) -> Option<usize> {
    let mut depth = 0;
    let mut ix = 0;
    while ix < glob.len() {
        match glob[ix] {
            '\\' => ix += 1,
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(ix + 1);
                }
            }
            _ => {}
        }
        ix += 1;
    }
    None
}

/// The inside of a `{…}`: a number range, alternatives split at the
/// commas outside nested braces, or with neither, the literal `{…}`.
fn braces(inner: &[char], ranges: &mut Vec<(i64, i64)>) -> String {
    let text: String = inner.iter().collect();
    if let Some((low, high)) = text.split_once("..")
        && let (Ok(low), Ok(high)) = (low.parse::<i64>(), high.parse::<i64>())
    {
        ranges.push((low.min(high), low.max(high)));
        return r"([+-]?\d+)".into();
    }
    let mut parts = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    let mut ix = 0;
    while ix < inner.len() {
        match inner[ix] {
            '\\' => ix += 1,
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(&inner[start..ix]);
                start = ix + 1;
            }
            _ => {}
        }
        ix += 1;
    }
    if parts.is_empty() {
        return format!(r"\{{{}\}}", translate(inner, ranges));
    }
    parts.push(&inner[start..]);
    let parts: Vec<String> = parts.into_iter().map(|p| translate(p, ranges)).collect();
    format!("(?:{})", parts.join("|"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(glob: &str, path: &str) -> bool {
        Pattern::new(glob).is_some_and(|p| p.matches(path))
    }

    #[test]
    fn globs() {
        assert!(matches("*", "a.rs"));
        assert!(matches("*", "src/deep/a.rs"), "no slash: any folder");
        assert!(matches("*.rs", "src/a.rs"));
        assert!(!matches("*.rs", "src/a.rsx"));
        assert!(matches("*.{yml,yaml,json}", "x/ci.yaml"));
        assert!(!matches("*.{yml,yaml}", "ci.toml"));
        assert!(matches("src/*.rs", "src/a.rs"));
        assert!(!matches("src/*.rs", "src/b/a.rs"), "* stays in a folder");
        assert!(!matches("src/*.rs", "lib/src/a.rs"), "a slash anchors it");
        assert!(matches("/src/*.rs", "src/a.rs"));
        assert!(matches("vendor/**", "vendor/a/b/c.c"));
        assert!(matches("a?.txt", "ab.txt"));
        assert!(matches("[abc].md", "b.md"));
        assert!(!matches("[!abc].md", "b.md"));
        assert!(matches("[!abc].md", "d.md"));
        assert!(matches("file{1..3}.txt", "file2.txt"));
        assert!(!matches("file{1..3}.txt", "file4.txt"));
        assert!(matches("{a,{b,c}}.x", "c.x"), "nested");
        assert!(matches("{single}.x", "{single}.x"), "no comma: literal");
        assert!(matches("[unclosed.x", "[unclosed.x"));
        assert!(matches(r"a\*.x", "a*.x"));
        assert!(!matches(r"a\*.x", "ab.x"));
    }

    fn write(dir: &Path, name: &str, text: &str) {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn nearer_files_and_later_sections_win() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".editorconfig",
            "root = true\n\n[*]\nindent_style = space\nindent_size = 4\n\
             trim_trailing_whitespace = true\n\n[*.md]\nindent_size = 2\n\
             trim_trailing_whitespace = false\n",
        );
        write(
            dir.path(),
            "web/.editorconfig",
            "# Not root: the one above still counts.\n[*.md]\nindent_style = TAB\n",
        );
        write(
            dir.path(),
            "vendor/.editorconfig",
            "[*]\nindent_size = unset\nend_of_line = crlf\n",
        );
        let detected = TabSize {
            tab_size: 8,
            hard_tabs: false,
        };

        let rust = EditorConfig::for_file(&dir.path().join("src/main.rs"));
        assert_eq!(rust.tab_size(detected).tab_size, 4);
        assert_eq!(rust.trim_trailing_whitespace(), Some(true));
        assert_eq!(rust.insert_final_newline(), None);

        let readme = EditorConfig::for_file(&dir.path().join("README.md"));
        assert_eq!(readme.tab_size(detected).tab_size, 2);
        assert_eq!(readme.trim_trailing_whitespace(), Some(false));

        let docs = EditorConfig::for_file(&dir.path().join("web/docs.md"));
        let tab = docs.tab_size(detected);
        assert!(tab.hard_tabs, "values are case-insensitive");
        assert_eq!(tab.tab_size, 2, "a tab's width follows indent_size");

        let vendored = EditorConfig::for_file(&dir.path().join("vendor/lib.c"));
        assert_eq!(vendored.tab_size(detected).tab_size, 8, "unset: detected");
        assert_eq!(vendored.end_of_line(), Some(LineEnding::CrLf));
    }

    #[test]
    fn root_stops_the_search() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".editorconfig",
            "root = true\n[*]\nindent_size = 3\n",
        );
        write(dir.path(), "inner/.editorconfig", "root = true\n");
        let config = EditorConfig::for_file(&dir.path().join("inner/a.rs"));
        assert_eq!(config, EditorConfig::default());
    }

    #[test]
    fn indent_size_tab_means_the_tab_width() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            ".editorconfig",
            "root = true\n[*.go]\nindent_style = tab\nindent_size = tab\ntab_width = 8\n",
        );
        let tab = EditorConfig::for_file(&dir.path().join("main.go")).tab_size(TabSize {
            tab_size: 4,
            hard_tabs: false,
        });
        assert_eq!((tab.tab_size, tab.hard_tabs), (8, true));
    }
}
