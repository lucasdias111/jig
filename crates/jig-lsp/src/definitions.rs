//! Guessing where a name is declared, for Cmd+click when no language server
//! answers (see [`crate::lsp`]), as Vim's `gd` does.
//!
//! Declarations are found by matching the
//! shapes they take across the bundled languages (`fn name`, `let name`,
//! `def name`, `name :=`, a parameter in a signature…). Locals and
//! parameters count only where they're in scope, which indentation tells.
//! Good enough for most jumps; it can be fooled by text in comments and
//! strings, or by a name declared in a shape it doesn't know.

use std::ops::Range;
use std::sync::LazyLock;

use regex::Regex;

/// Words that are never a jump target.
const KEYWORDS: &[&str] = &[
    "as",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "def",
    "default",
    "do",
    "elif",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "final",
    "finally",
    "fn",
    "for",
    "from",
    "func",
    "function",
    "go",
    "if",
    "impl",
    "import",
    "in",
    "interface",
    "is",
    "lambda",
    "let",
    "loop",
    "match",
    "mod",
    "mut",
    "new",
    "nil",
    "None",
    "not",
    "null",
    "or",
    "and",
    "package",
    "pass",
    "private",
    "protected",
    "pub",
    "public",
    "raise",
    "ref",
    "return",
    "self",
    "Self",
    "static",
    "struct",
    "super",
    "switch",
    "this",
    "throw",
    "trait",
    "true",
    "True",
    "False",
    "try",
    "type",
    "undefined",
    "use",
    "var",
    "void",
    "where",
    "while",
    "with",
    "yield",
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// Visible from anywhere: functions, types, constants.
    Item,
    /// Visible until the block it's declared in closes.
    Local,
    /// Visible in the body that follows: parameters.
    Param,
    /// Visible in the loop's body.
    Loop,
    /// Reached as `.name` or `::Name`: fields, enum variants.
    Member,
}

#[derive(Clone, Debug)]
struct Declaration {
    kind: Kind,
    /// The name itself.
    range: Range<usize>,
}

/// What Cmd+clicking the name at an offset should do.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    /// Go to the declaration at this range in the same text.
    Here(Range<usize>),
    /// The name clicked is a declaration: show where it's used.
    Usages,
    /// Not declared in this text; look in other files with
    /// [`declaration_in_file`].
    Elsewhere,
}

/// The name at `offset`, if it's one worth jumping from.
pub fn name_at(text: &str, offset: usize) -> Option<Range<usize>> {
    let is_name = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    let offset = offset.min(text.len());
    if !text.is_char_boundary(offset) {
        return None;
    }
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_name(*c))
        .last()
        .map_or(offset, |(ix, _)| ix);
    let end = text[offset..]
        .char_indices()
        .find(|(_, c)| !is_name(*c))
        .map_or(text.len(), |(ix, _)| offset + ix);
    let name = &text[start..end];
    let first = name.chars().next()?;
    (!first.is_ascii_digit() && !KEYWORDS.contains(&name)).then_some(start..end)
}

/// Where the name at `range` in `text` is declared.
pub fn resolve(text: &str, range: Range<usize>, language: &str) -> Target {
    let name = &text[range.clone()];
    let declarations = declarations(text, name, language);
    if declarations.iter().any(|d| d.range == range) {
        return Target::Usages;
    }

    if is_member_access(text, range.start) {
        // `x.name`: which `x` isn't known, so take the first field or item
        // by that name.
        return declarations
            .iter()
            .find(|d| matches!(d.kind, Kind::Member | Kind::Item))
            .map_or(Target::Elsewhere, |d| Target::Here(d.range.clone()));
    }

    // The innermost local in scope, else the item nearest above, else the
    // first item below.
    let local = declarations
        .iter()
        .filter(|d| matches!(d.kind, Kind::Local | Kind::Param | Kind::Loop))
        .filter(|d| d.range.start < range.start && in_scope(text, d, range.start))
        .max_by_key(|d| d.range.start);
    let items = || declarations.iter().filter(|d| d.kind == Kind::Item);
    let item = items()
        .filter(|d| d.range.start < range.start)
        .max_by_key(|d| d.range.start)
        .or_else(|| items().next());
    local
        .or(item)
        .map_or(Target::Elsewhere, |d| Target::Here(d.range.clone()))
}

/// Where `name` is declared at the top level of another file, for a name
/// [`resolve`] found nothing for. `member` is whether it was reached as
/// `.name` or `::Name`, and so may be a field or variant.
pub fn declaration_in_file(
    text: &str,
    name: &str,
    language: &str,
    member: bool,
) -> Option<Range<usize>> {
    declarations(text, name, language)
        .into_iter()
        .find(|d| d.kind == Kind::Item || (member && d.kind == Kind::Member))
        .map(|d| d.range)
}

/// Whether the name starting at `start` follows a `.` or `::`.
pub fn is_member_access(text: &str, start: usize) -> bool {
    let before = text[..start].trim_end_matches([' ', '\t']);
    before.ends_with('.') && !before.ends_with("..") || before.ends_with("::")
}

/// The next whole-word occurrence of the name at `range`, wrapping around.
pub fn next_occurrence(text: &str, range: Range<usize>) -> Option<Range<usize>> {
    let pattern = Regex::new(&format!(r"\b{}\b", regex::escape(&text[range.clone()]))).ok()?;
    let found: Vec<_> = pattern.find_iter(text).map(|m| m.range()).collect();
    found
        .iter()
        .find(|r| r.start > range.start)
        .or_else(|| found.first())
        .filter(|r| **r != range)
        .cloned()
}

fn declarations(text: &str, name: &str, language: &str) -> Vec<Declaration> {
    let mut found: Vec<Declaration> = Vec::new();
    for (kind, pattern) in patterns(name, language) {
        let Ok(regex) = Regex::new(&pattern) else {
            continue;
        };
        for captures in regex.captures_iter(text) {
            let Some(m) = captures.name("name") else {
                continue;
            };
            let range = m.range();
            if kind == Kind::Param && !is_signature_line(text, range.start) {
                continue;
            }
            // Python: an assignment at the top level is a module item.
            let kind = if kind == Kind::Local
                && language == "python"
                && indentation(line_at(text, range.start)) == 0
            {
                Kind::Item
            } else {
                kind
            };
            if !found.iter().any(|d| d.range == range) {
                found.push(Declaration { kind, range });
            }
        }
    }
    found.sort_by_key(|d| d.range.start);
    found
}

/// The shapes a declaration of `name` takes, each with the name captured as
/// `name`.
fn patterns(name: &str, language: &str) -> Vec<(Kind, String)> {
    let n = format!("(?P<name>{})", regex::escape(name));
    let mut patterns = vec![
        (
            Kind::Item,
            format!(
                r"\b(?:fn|struct|enum|union|trait|type|mod|macro_rules!|class|interface|def|function\*?|const|static|namespace|record)\s+(?:mut\s+)?{n}\b"
            ),
        ),
        // Go: `func Name(` and methods, `func (r *T) Name(`.
        (Kind::Item, format!(r"\bfunc\s*(?:\([^)]*\)\s*)?{n}\b")),
        (
            Kind::Local,
            format!(r"\b(?:let|var)\s+(?:mut\s+|ref\s+)?{n}\b"),
        ),
        // Tuples and destructuring: `let (a, name) =`, `const { a, name } =`.
        (
            Kind::Local,
            format!(
                r"\b(?:let|const|var)\s+(?:mut\s+)?[(\[{{]\s*(?:[\w\s:]+,\s*)*(?:mut\s+)?{n}\s*[,)\]}}=]"
            ),
        ),
        (
            Kind::Loop,
            format!(
                r"\bfor\s*\(?\s*(?:(?:const|let|var|mut)\s+)?(?:\(?\s*(?:mut\s+)?\w+\s*,\s*)*(?:mut\s+)?{n}\b[^\n]*?\b(?:in|of)\b"
            ),
        ),
        // Parameters, on a line that starts a function or closure.
        (
            Kind::Param,
            format!(r"[(,|]\s*(?:mut\s+|&\s*|\*\s*|\.\.\.)?{n}\s*(?:[:,)=|?]|\s+[\w\[*])"),
        ),
        (Kind::Param, format!(r"\b{n}\s*=>")),
        (Kind::Param, format!(r"\blambda\s+(?:\w+\s*,\s*)*{n}\b")),
    ];
    match language {
        "rust" => patterns.extend([
            // Struct fields.
            (
                Kind::Member,
                format!(r"(?m)^[ \t]*(?:pub(?:\([^)]*\))?\s+)?{n}\s*:[^:]"),
            ),
        ]),
        "go" => patterns.extend([
            (Kind::Local, format!(r"\b{n}(?:\s*,\s*\w+)*\s*:=")),
            (Kind::Local, format!(r"\b\w+(?:\s*,\s*\w+)*\s*,\s*{n}\s*:=")),
            (Kind::Member, format!(r"(?m)^[ \t]*{n}\s+[\w*\[]")),
        ]),
        "python" => patterns.extend([
            (
                Kind::Local,
                format!(r"(?m)^[ \t]*(?:\w+\s*,\s*)*{n}\s*(?:,\s*\w+\s*)*(?::[^=\n]+)?=[^=]"),
            ),
            (Kind::Local, format!(r"\bas\s+{n}\b")),
            (
                Kind::Member,
                format!(r"\bself\.{n}\s*(?::[^=\n]+)?=[^=]"),
            ),
        ]),
        "typescript" | "tsx" | "javascript" | "java" => patterns.extend([
            // Class methods: `async name(a) {`, `public void name(int a) {`.
            (
                Kind::Item,
                format!(r"(?m)^[ \t]*(?:(?:public|private|protected|static|async|readonly|override|abstract|final|synchronized|get|set)\s+)*(?:[\w<>\[\],.?]+\s+)?{n}\s*(?:<[^>\n]*>)?\s*\([^;\n]*\{{\s*$"),
            ),
            // Class fields and object properties.
            (
                Kind::Member,
                format!(r"(?m)^[ \t]*(?:(?:public|private|protected|static|readonly|final)\s+)*{n}\s*[?!]?\s*[:=][^=]"),
            ),
        ]),
        _ => {}
    }
    if language == "rust" && name.starts_with(char::is_uppercase) {
        // Enum variants.
        patterns.push((Kind::Member, format!(r"(?m)^[ \t]*{n}\s*(?:[({{,]|$)")));
    }
    if language == "java" {
        // Typed declarations: `String name =`, `List<T> name;`, `int name)`.
        patterns.push((
            Kind::Local,
            format!(r"\b(?:[A-Z]\w*|int|long|short|byte|char|boolean|float|double|var)(?:<[^;=()\n]*>)?(?:\[\])*\s+{n}\s*(?:=[^=]|[;,):])"),
        ));
    }
    patterns
}

/// Whether the line holding `offset` declares a function or closure, so a
/// name in parentheses there is a parameter rather than an argument.
fn is_signature_line(text: &str, offset: usize) -> bool {
    static SIGNATURE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"\b(?:fn|def|function|func|lambda)\b|=>|(?:^|[(=,]\s*|move\s+)\|[^|]*\|")
            .unwrap()
    });
    // A method without a keyword: `name(a, b) {`, `void name(int a) {`.
    static METHOD: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"^\s*(?:[\w<>\[\],.?]+\s+)*(?P<word>\w+)\s*(?:<[^>]*>)?\s*\(.*\)[^{;]*\{\s*$")
            .unwrap()
    });
    const CONTROL: &[&str] = &["if", "for", "while", "switch", "catch", "with", "return"];
    let line = line_at(text, offset);
    SIGNATURE.is_match(line)
        || METHOD
            .captures(line)
            .is_some_and(|c| !CONTROL.contains(&&c["word"]))
}

/// Whether `decl` is still in scope at `offset`: no line between them
/// closes the block it was declared in.
fn in_scope(text: &str, decl: &Declaration, offset: usize) -> bool {
    let line = line_at(text, decl.range.start);
    let indent = indentation(line);
    let line_end = text[decl.range.start..]
        .find('\n')
        .map_or(text.len(), |ix| decl.range.start + ix);
    let Some(between) = text.get(line_end..offset) else {
        return true;
    };
    // The usage's own line counts only up to the usage.
    between.split('\n').skip(1).all(|line| {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            return true;
        }
        let level = indentation(line);
        match decl.kind {
            // A parameter's body is indented deeper than its signature;
            // getting back to the signature's level ends it, except for
            // the rest of a signature split over lines.
            Kind::Param | Kind::Loop => {
                level > indent || (level == indent && trimmed.starts_with([')', '{']))
            }
            _ => level >= indent,
        }
    })
}

fn line_at(text: &str, offset: usize) -> &str {
    let start = text[..offset].rfind('\n').map_or(0, |ix| ix + 1);
    let end = text[offset..]
        .find('\n')
        .map_or(text.len(), |ix| offset + ix);
    &text[start..end]
}

/// Leading whitespace, with a tab as four columns.
fn indentation(line: &str) -> usize {
    line.chars()
        .take_while(|c| c.is_whitespace())
        .map(|c| if c == '\t' { 4 } else { 1 })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Resolve the `n`th (from 0) occurrence of `name`, and describe the
    /// result as the occurrence index it lands on.
    fn jump(text: &str, language: &str, name: &str, n: usize) -> Option<usize> {
        let occurrences: Vec<usize> = Regex::new(&format!(r"\b{name}\b"))
            .unwrap()
            .find_iter(text)
            .map(|m| m.start())
            .collect();
        let start = occurrences[n];
        let range = name_at(text, start + 1).unwrap();
        assert_eq!(&text[range.clone()], name);
        match resolve(text, range, language) {
            Target::Here(target) => occurrences.iter().position(|&o| o == target.start),
            Target::Usages => Some(usize::MAX),
            Target::Elsewhere => None,
        }
    }

    const USAGES: Option<usize> = Some(usize::MAX);

    #[test]
    fn names() {
        let text = "let foo_bar = 1 + x2;";
        assert_eq!(name_at(text, 6), Some(4..11));
        assert_eq!(name_at(text, 11), Some(4..11));
        assert_eq!(name_at(text, 1), None, "keyword");
        assert_eq!(name_at(text, 14), None, "number");
        assert_eq!(name_at(text, 19), Some(18..20));
    }

    #[test]
    fn rust_functions_and_locals() {
        let text = "\
fn helper(count: usize) -> usize {
    let total = count * 2;
    total
}

fn main() {
    let total = helper(3);
    println!(\"{total}\");
}
";
        assert_eq!(jump(text, "rust", "helper", 1), Some(0));
        assert_eq!(jump(text, "rust", "helper", 0), USAGES);
        assert_eq!(jump(text, "rust", "count", 1), Some(0));
        assert_eq!(jump(text, "rust", "total", 1), Some(0));
        // The `total` in main is main's, not helper's.
        assert_eq!(jump(text, "rust", "total", 3), Some(2));
    }

    #[test]
    fn parameters_end_with_their_function() {
        let text = "\
fn a(value: u8) {
    value;
}
fn b() {
    value;
}
";
        assert_eq!(jump(text, "rust", "value", 1), Some(0));
        assert_eq!(jump(text, "rust", "value", 2), None);
    }

    #[test]
    fn rust_fields_and_variants() {
        let text = "\
struct Point {
    pub x: f32,
}
enum Shape {
    Circle(f32),
    Square,
}
fn f(p: Point) -> Shape {
    let _ = p.x;
    Shape::Square
}
";
        assert_eq!(jump(text, "rust", "x", 1), Some(0));
        assert_eq!(jump(text, "rust", "Square", 1), Some(0));
        assert_eq!(jump(text, "rust", "Point", 1), Some(0));
        assert_eq!(jump(text, "rust", "Shape", 2), Some(0));
    }

    #[test]
    fn closures_and_loops() {
        let text = "\
fn f(items: Vec<u8>) {
    for item in items {
        item;
    }
    items.iter().map(|each| each + 1);
}
";
        assert_eq!(jump(text, "rust", "item", 1), Some(0));
        assert_eq!(jump(text, "rust", "each", 1), Some(0));
    }

    #[test]
    fn python() {
        let text = "\
LIMIT = 3

class Box:
    def __init__(self, size):
        self.size = size

    def grow(self, by):
        total = self.size + by
        return min(total, LIMIT)
";
        assert_eq!(jump(text, "python", "LIMIT", 1), Some(0));
        assert_eq!(jump(text, "python", "size", 3), Some(1), "attribute");
        assert_eq!(jump(text, "python", "size", 2), Some(0), "parameter");
        assert_eq!(jump(text, "python", "by", 1), Some(0));
        assert_eq!(jump(text, "python", "total", 1), Some(0));
        assert_eq!(jump(text, "python", "Box", 0), USAGES);
    }

    #[test]
    fn typescript() {
        let text = "\
const limit = 3;
export function clamp(value: number): number {
  return Math.min(value, limit);
}
class Counter {
  count = 0;
  bump(by: number) {
    this.count += by;
    return clamp(this.count);
  }
}
";
        assert_eq!(jump(text, "typescript", "limit", 1), Some(0));
        assert_eq!(jump(text, "typescript", "value", 1), Some(0));
        assert_eq!(jump(text, "typescript", "clamp", 1), Some(0));
        assert_eq!(jump(text, "typescript", "count", 1), Some(0));
        assert_eq!(jump(text, "typescript", "by", 1), Some(0));
        assert_eq!(jump(text, "typescript", "bump", 0), USAGES);
    }

    #[test]
    fn go() {
        let text = "\
func (s *Server) Start(port int) error {
\taddr, err := listen(port)
\treturn use(addr, err)
}
";
        assert_eq!(jump(text, "go", "port", 1), Some(0));
        assert_eq!(jump(text, "go", "addr", 1), Some(0));
        assert_eq!(jump(text, "go", "err", 1), Some(0));
        assert_eq!(jump(text, "go", "Start", 0), USAGES);
        assert_eq!(jump(text, "go", "listen", 0), None);
    }

    #[test]
    fn java() {
        let text = "\
class Greeter {
    private String name;
    public String greet(String prefix) {
        String line = prefix + name;
        return line;
    }
}
";
        assert_eq!(jump(text, "java", "name", 1), Some(0));
        assert_eq!(jump(text, "java", "prefix", 1), Some(0));
        assert_eq!(jump(text, "java", "line", 1), Some(0));
        assert_eq!(jump(text, "java", "greet", 0), USAGES);
    }

    #[test]
    fn other_files() {
        let text = "pub struct Config {\n    pub name: String,\n}\n";
        assert_eq!(
            declaration_in_file(text, "Config", "rust", false),
            Some(11..17)
        );
        assert_eq!(declaration_in_file(text, "name", "rust", false), None);
        assert_eq!(
            declaration_in_file(text, "name", "rust", true),
            Some(28..32)
        );
    }

    #[test]
    fn next_occurrences_wrap() {
        let text = "a b a c a";
        assert_eq!(next_occurrence(text, 4..5), Some(8..9));
        assert_eq!(next_occurrence(text, 8..9), Some(0..1));
        assert_eq!(next_occurrence("only", 0..4), None);
    }
}
