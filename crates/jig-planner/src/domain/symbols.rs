//! The names a task can link to: what's declared across the project, and
//! its files, for the `@` list in the task form. Found by the shapes
//! declarations take (`fn name`, `class Name`, `func name`, a top-level
//! `const NAME`…) rather than by language servers, so it works for every
//! language, with or without a server installed. Locals are left out.
//!
//! Reading the project is the store's ([`ProjectFiles`]), so this works on
//! whatever files it's handed.

use std::path::Path;
use std::sync::LazyLock;

use regex::Regex;

/// Bigger files are generated or data, not code worth linking.
const MAX_FILE_BYTES: u64 = 512 * 1024;
/// Enough for any project anyone links from a task.
const MAX_SYMBOLS: usize = 100_000;

/// Files that hold code, by extension.
const CODE: &[&str] = &[
    "rs", "go", "py", "pyi", "js", "mjs", "cjs", "jsx", "ts", "mts", "cts", "tsx", "gd", "java",
    "kt", "kts", "swift", "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "cs", "rb", "php", "lua",
    "zig", "ex", "exs", "scala", "dart", "m", "mm", "vue", "svelte", "sh", "bash", "nim", "ml",
    "hs", "elm", "jl", "cr", "odin", "gleam",
];

/// One declared name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    /// The word that declares it, e.g. `fn` or `class`.
    pub kind: String,
    /// Relative to the project.
    pub path: String,
    /// From 1.
    pub line: usize,
}

/// What `@` can link to in a project.
#[derive(Clone, Debug, Default)]
pub struct ProjectIndex {
    pub symbols: Vec<Symbol>,
    /// Relative to the project.
    pub files: Vec<String>,
}

/// A declaration: decorators and modifiers, the declaring word, a Go
/// method's receiver, then the name.
static DECLARATION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^(?P<indent>[ \t]*)(?:@\w+(?:\([^)\n]*\))?\s+)*(?:(?:pub(?:\([^)\n]*\))?|export|default|async|static|public|private|protected|internal|abstract|final|override|unsafe|extern|inline|open|data|sealed|readonly|local|const)\s+)*(?P<kind>fn|fun|func|function|def|class|class_name|struct|enum|trait|interface|type|mod|module|protocol|object|record|signal|let|var|val|const|static)\s+(?:mut\s+)?(?:\([^)\n]*\)\s*)?(?P<name>[A-Za-z_$][\w$]*)",
    )
    .expect("the declaration pattern")
});

/// Words that declare variables, which count only at the top level, where
/// they're module constants and members rather than locals.
const VARIABLES: &[&str] = &["let", "var", "val", "const", "static"];

/// The declarations in `text`: kind, name and line, in order.
pub fn declarations(text: &str) -> Vec<(String, String, usize)> {
    let mut line = 1;
    let mut counted = 0;
    DECLARATION
        .captures_iter(text)
        .filter(|captures| !VARIABLES.contains(&&captures["kind"]) || captures["indent"].is_empty())
        .map(|captures| {
            let at = captures.get(0).map_or(0, |m| m.start());
            line += text[counted..at].matches('\n').count();
            counted = at;
            (
                captures["kind"].to_string(),
                captures["name"].to_string(),
                line,
            )
        })
        .collect()
}

/// The project's files, as the index reads them. The store reads them
/// from disk; tests hand them over in memory.
pub trait ProjectFiles {
    /// Every file, relative to the project, as `.gitignore` leaves them.
    fn files(&self) -> Vec<String>;
    /// How many bytes the file at `path` holds, if it's there.
    fn size(&self, path: &str) -> Option<u64>;
    /// The file at `path` as text, if it reads as text.
    fn read(&self, path: &str) -> Option<String>;
}

/// Read what `project` declares. Reads every file; run it in the background.
pub fn index(project: &impl ProjectFiles) -> ProjectIndex {
    // Jig's own files, the tasks among them, aren't what a task links to.
    let files: Vec<String> = project
        .files()
        .into_iter()
        .filter(|path| !path.starts_with(".jig/"))
        .collect();
    let mut symbols = Vec::new();
    for path in &files {
        let code = Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| CODE.contains(&extension));
        let small = project
            .size(path)
            .is_some_and(|size| size <= MAX_FILE_BYTES);
        if !code || !small {
            continue;
        }
        let Some(text) = project.read(path) else {
            continue;
        };
        symbols.extend(
            declarations(&text)
                .into_iter()
                .map(|(kind, name, line)| Symbol {
                    name,
                    kind,
                    path: path.clone(),
                    line,
                }),
        );
        if symbols.len() >= MAX_SYMBOLS {
            break;
        }
    }
    ProjectIndex { symbols, files }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(text: &str) -> Vec<(String, usize)> {
        declarations(text)
            .into_iter()
            .map(|(_, name, line)| (name, line))
            .collect()
    }

    #[test]
    fn finds_declarations_across_languages() {
        let rust = "pub struct Planner;\n\nimpl Planner {\n    pub(crate) fn reload(&self) {\n        let local = 1;\n    }\n}\n\nconst LIMIT: usize = 3;\nasync fn fetch() {}\n";
        assert_eq!(
            names(rust),
            [
                ("Planner".into(), 1),
                ("reload".into(), 4),
                ("LIMIT".into(), 9),
                ("fetch".into(), 10)
            ],
            "the local is left out"
        );
        let gdscript = "class_name PlayerController\nextends CharacterBody3D\n\nsignal overheated\n@export var speed := 4.0\nvar heat = 0\n\nfunc _add_heat(amount):\n\tvar total = heat\n";
        assert_eq!(
            names(gdscript),
            [
                ("PlayerController".into(), 1),
                ("overheated".into(), 4),
                ("speed".into(), 5),
                ("heat".into(), 6),
                ("_add_heat".into(), 8)
            ]
        );
        let go = "type Server struct{}\n\nfunc (s *Server) Start() {}\nfunc main() {}\n";
        assert_eq!(
            names(go),
            [
                ("Server".into(), 1),
                ("Start".into(), 3),
                ("main".into(), 4)
            ]
        );
        let ts = "export default function App() {}\nexport const routes = [];\nclass Cache {\n  private get(key) {}\n}\n";
        assert_eq!(
            names(ts),
            [("App".into(), 1), ("routes".into(), 2), ("Cache".into(), 3)]
        );
        assert_eq!(
            names("class Parser:\n    def parse(self):\n        pass\n"),
            [("Parser".into(), 1), ("parse".into(), 2)]
        );
    }

    /// Files in memory, by path.
    struct Files(Vec<(&'static str, &'static str)>);

    impl ProjectFiles for Files {
        fn files(&self) -> Vec<String> {
            self.0.iter().map(|(path, _)| path.to_string()).collect()
        }

        fn size(&self, path: &str) -> Option<u64> {
            self.read(path).map(|text| text.len() as u64)
        }

        fn read(&self, path: &str) -> Option<String> {
            self.0
                .iter()
                .find(|(p, _)| *p == path)
                .map(|(_, text)| text.to_string())
        }
    }

    #[test]
    fn indexes_a_projects_code_and_files() {
        let index = index(&Files(vec![
            (".jig/tasks/a.md", "# A\n"),
            ("README.md", "type the name\n"),
            ("src/a.rs", "fn run() {}\n"),
        ]));
        assert_eq!(
            index.symbols,
            [Symbol {
                name: "run".into(),
                kind: "fn".into(),
                path: "src/a.rs".into(),
                line: 1
            }],
            "prose isn't code"
        );
        assert_eq!(index.files, ["README.md", "src/a.rs"]);
    }
}
