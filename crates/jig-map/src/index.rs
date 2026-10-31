//! The project read for the Map: every file the outline reads, its
//! functions and what each one calls, and the files it can't read (a
//! Svelte page, a template), kept by their folder so every file has a
//! place. Calls are resolved without a language server, by name, narrowed
//! by what they're called on (its declared type, an import, the class
//! they're in): one match is a call, a few are ambiguous, a type from
//! outside the project is a call into that package. A language server,
//! when it answers, does better for the function in focus (see
//! `workspace/map.rs`).
//!
//! It's read on a background thread, and read again file by file: a file
//! unchanged since the last read keeps what it read as.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::outline::{self, Call, FunctionOutline, Import, Lang, Outline, TypeDecl, Variable};

mod reading;
mod resolving;
mod walk;
use walk::{MANIFESTS, aliases, source_files};

/// A function called from this many others is a hub: logging, a utility.
pub const HUB_CALLERS: usize = 12;
/// More candidates than this for a name, and a call isn't drawn at all.
const MAX_AMBIGUOUS: usize = 3;

pub type FnId = usize;

/// A source file, relative to the project.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub path: PathBuf,
    pub lang: Lang,
    /// What it's for, in a line, from its doc comment.
    pub purpose: Option<String>,
    /// When it was last changed and how long it was, as read.
    pub stamp: Option<Stamp>,
    pub module: usize,
    pub package: Option<String>,
    pub imports: Vec<Import>,
    pub types: Vec<TypeDecl>,
    /// Its types' fields and its own variables.
    pub variables: Vec<Variable>,
}

/// A folder of source files, or a Java package.
#[derive(Clone, Debug)]
pub struct Module {
    /// The folder, relative to the project.
    pub dir: PathBuf,
    /// As it's shown: the package past what all share, or the folder.
    pub label: String,
    pub files: Vec<usize>,
    pub functions: Vec<FnId>,
}

#[derive(Clone, Debug)]
pub struct Func {
    pub file: usize,
    pub outline: FunctionOutline,
}

impl Func {
    pub fn short(&self) -> &str {
        &self.outline.function.short
    }

    /// `Owner.name`, or the name.
    pub fn title(&self) -> String {
        match &self.outline.owner {
            Some(owner) => format!("{owner}.{}", self.short()),
            None => self.short().to_string(),
        }
    }

    pub fn range(&self) -> &Range<usize> {
        &self.outline.function.range
    }
}

/// What a call reaches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Function(FnId),
    /// One of these, by name alone.
    Ambiguous(Vec<FnId>),
    /// A package outside the project.
    External(String),
    /// A function value: whatever the caller was handed.
    Unknown,
}

/// One call, or callback, from a function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Edge {
    pub from: FnId,
    pub target: Target,
    /// Which of its calls (into `outline.calls`).
    pub call: usize,
    /// A function handed on, not called.
    pub callback: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Project {
    pub root: PathBuf,
    pub files: Vec<SourceFile>,
    pub modules: Vec<Module>,
    pub functions: Vec<Func>,
    /// Each function's calls, resolved.
    pub edges: Vec<Vec<Edge>>,
    /// Each function's callers.
    pub callers: Vec<Vec<FnId>>,
    /// Each module's imports of other modules, with how many.
    pub imports: Vec<Vec<(usize, usize)>>,
    /// Files the outline can't read, relative to the project, each placed
    /// by its folder.
    pub others: Vec<PathBuf>,
    /// Folders a manifest makes a package or a service (`Cargo.toml`,
    /// `package.json`, `go.mod`…), relative to the project.
    pub packages: Vec<PathBuf>,
    /// What folders are for, in a line, from their READMEs.
    pub readmes: HashMap<PathBuf, String>,
    /// Files there were too many of to read.
    pub skipped: usize,
    /// Functions the language server answered for.
    checked: HashSet<FnId>,
    /// TypeScript's path aliases, from each `tsconfig*.json`.
    pub aliases: Vec<Alias>,
    by_short: HashMap<String, Vec<FnId>>,
    /// Each type's declarations: (file, index into its types).
    types: HashMap<String, Vec<(usize, usize)>>,
}

/// The language a file is read as, by its name.
pub fn lang_of(path: &Path) -> Option<Lang> {
    let name = path.file_name()?.to_str()?;
    if name.ends_with(".d.ts") {
        return None;
    }
    Some(match path.extension()?.to_str()? {
        "rs" => Lang::Rust,
        "java" => Lang::Java,
        "ts" | "mts" | "cts" => Lang::TypeScript,
        "tsx" => Lang::Tsx,
        "js" | "mjs" | "cjs" | "jsx" => Lang::JavaScript,
        "py" => Lang::Python,
        "go" => Lang::Go,
        "c" | "h" => Lang::C,
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => Lang::Cpp,
        "swift" => Lang::Swift,
        "zig" => Lang::Zig,
        _ => return None,
    })
}

/// The project `file` is in, when no folder is open: the nearest folder
/// above it with `.git`, else with a manifest (`Cargo.toml`,
/// `package.json`…), else its own folder.
pub fn root_of(file: &Path) -> PathBuf {
    let dir = file.parent().unwrap_or(Path::new("."));
    let with = |marker: &dyn Fn(&Path) -> bool| dir.ancestors().find(|dir| marker(dir));
    with(&|dir| dir.join(".git").exists())
        .or_else(|| with(&|dir| MANIFESTS.iter().any(|name| dir.join(name).is_file())))
        .unwrap_or(dir)
        .to_path_buf()
}

/// When a file was last changed, and how long it was.
pub type Stamp = (SystemTime, u64);

impl Project {
    /// What the language server says function `id` calls and is called
    /// by, over what the index guessed: certain, where it answered.
    pub fn merge_server(&mut self, id: FnId, callees: Vec<Target>, callers: Vec<FnId>) {
        let mut edges: Vec<Edge> = callees
            .into_iter()
            .map(|target| Edge {
                from: id,
                target,
                call: 0,
                callback: false,
            })
            .collect();
        // Callbacks and unknown targets the server doesn't report.
        edges.extend(self.edges[id].iter().filter(|edge| edge.callback).cloned());
        edges.dedup();
        for list in &mut self.callers {
            list.retain(|&caller| caller != id);
        }
        for edge in &edges {
            if let Target::Function(to) = edge.target
                && to != id
                && !self.callers[to].contains(&id)
            {
                self.callers[to].push(id);
            }
        }
        self.edges[id] = edges;
        for caller in callers {
            if caller != id && !self.callers[id].contains(&caller) {
                self.callers[id].push(caller);
            }
        }
        self.checked.insert(id);
    }

    /// Whether the language server has answered for function `id`.
    pub fn checked(&self, id: FnId) -> bool {
        self.checked.contains(&id)
    }

    /// Whether function `id` is a test, or test support: in a test folder
    /// or file.
    pub fn is_test(&self, id: FnId) -> bool {
        let path = self.files[self.functions[id].file]
            .path
            .to_string_lossy()
            .to_lowercase();
        path.contains("/test/")
            || path.contains("/tests/")
            || path.contains("/e2e/")
            || path.contains("/__tests__/")
            || path.contains(".test.")
            || path.contains(".spec.")
            || path.ends_with("test.java")
            || path.ends_with("tests.java")
    }

    /// Whether function `id` is called from so many places it's a hub.
    pub fn is_hub(&self, id: FnId) -> bool {
        self.callers[id].len() >= HUB_CALLERS
    }

    /// The implementations of abstract function `id`: methods of the same
    /// name in types implementing its type.
    pub fn implementations(&self, id: FnId) -> Vec<FnId> {
        let function = &self.functions[id];
        let Some(owner) = function.outline.owner.as_deref() else {
            return Vec::new();
        };
        if !function.outline.is_abstract {
            return Vec::new();
        }
        let implementers: Vec<&str> = self
            .files
            .iter()
            .flat_map(|file| &file.types)
            .filter(|ty| ty.implements.iter().any(|i| i == owner))
            .map(|ty| ty.name.as_str())
            .collect();
        self.named(function.short())
            .into_iter()
            .filter(|&other| {
                self.functions[other]
                    .outline
                    .owner
                    .as_deref()
                    .is_some_and(|o| implementers.contains(&o))
            })
            .collect()
    }

    /// The functions function `id` calls, each once, certain ones only.
    pub fn callees(&self, id: FnId) -> Vec<FnId> {
        let mut found = Vec::new();
        for edge in &self.edges[id] {
            if let Target::Function(to) = edge.target
                && to != id
                && !found.contains(&to)
            {
                found.push(to);
            }
        }
        found
    }

    /// The function around `offset` in `file` (relative), innermost.
    pub fn function_at(&self, file: &Path, offset: usize) -> Option<FnId> {
        let file = self.files.iter().position(|f| f.path == file)?;
        self.functions
            .iter()
            .enumerate()
            .filter(|(_, f)| f.file == file && f.range().contains(&offset))
            .min_by_key(|(_, f)| f.range().len())
            .map(|(id, _)| id)
    }

    /// The function around line `line` (0-based) of `file` (relative),
    /// innermost.
    pub fn function_on_line(&self, file: &Path, line: u32) -> Option<FnId> {
        let file = self.files.iter().position(|f| f.path == file)?;
        self.functions
            .iter()
            .enumerate()
            .filter(|(_, f)| f.file == file && f.outline.lines.contains(&line))
            .min_by_key(|(_, f)| f.outline.lines.len())
            .map(|(id, _)| id)
    }

    /// The path that keeps function `id` across reads: its file, owner,
    /// name and how many parameters it takes.
    pub fn symbol(&self, id: FnId) -> Symbol {
        let function = &self.functions[id];
        Symbol {
            file: self.files[function.file]
                .path
                .to_string_lossy()
                .replace('\\', "/"),
            owner: function.outline.owner.clone(),
            name: function.short().to_string(),
            arity: function.outline.parameters.len(),
        }
    }

    /// The function `symbol` names, found again after edits: the same
    /// place, else renamed files or types, else by name if there's one.
    pub fn find(&self, symbol: &Symbol) -> Option<FnId> {
        let matches = |id: &FnId, file: bool, owner: bool, arity: bool| {
            let f = &self.functions[*id];
            f.short() == symbol.name
                && (!file
                    || self.files[f.file].path.to_string_lossy().replace('\\', "/") == symbol.file)
                && (!owner || f.outline.owner == symbol.owner)
                && (!arity || f.outline.parameters.len() == symbol.arity)
        };
        let named = self.named(&symbol.name);
        for (file, owner, arity) in [
            (true, true, true),
            (true, true, false),
            (false, true, true),
            (true, false, true),
        ] {
            let found: Vec<FnId> = named
                .iter()
                .copied()
                .filter(|id| matches(id, file, owner, arity))
                .collect();
            if let [one] = found.as_slice() {
                return Some(*one);
            }
            if !found.is_empty() && file && owner {
                return found.first().copied();
            }
        }
        (named.len() == 1).then(|| named[0])
    }
}

/// A TypeScript path alias: under `dir`, imports starting `prefix` are
/// files under `target` (both relative to the project).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    pub dir: PathBuf,
    pub prefix: String,
    pub target: PathBuf,
}

/// A function's lasting name, as places and plans keep it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Symbol {
    pub file: String,
    pub owner: Option<String>,
    pub name: String,
    pub arity: usize,
}

impl Symbol {
    pub fn title(&self) -> String {
        match &self.owner {
            Some(owner) => format!("{owner}.{}", self.name),
            None => self.name.clone(),
        }
    }
}

/// `a/b/../c` as `a/c`.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// A project to read in tests, here and in the app's.
#[cfg(any(test, feature = "testing"))]
pub mod fixture;

#[cfg(test)]
pub mod tests;
