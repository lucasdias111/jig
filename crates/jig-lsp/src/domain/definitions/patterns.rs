//! The shapes declarations take in each language, as regular
//! expressions, and the words that never name one.

use super::Kind;

/// Words that are never a jump target.
pub(super) const KEYWORDS: &[&str] = &[
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

/// The shapes a declaration of `name` takes, each with the name captured as
/// `name`.
pub(super) fn patterns(name: &str, language: &str) -> Vec<(Kind, String)> {
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
