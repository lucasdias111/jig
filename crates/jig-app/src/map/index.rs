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

/// Folders never read: dependencies, build output, version control.
const SKIPPED: &[&str] = &[
    "node_modules",
    "target",
    "build",
    "dist",
    "out",
    "bin",
    "coverage",
    "vendor",
    "__pycache__",
    "venv",
];
/// More than this and the rest of the project isn't read.
const MAX_FILES: usize = 12000;
/// Files the outline can't read but that are the project's own code or
/// pages, placed by their folder: templates, styles, other languages.
const OTHER_SOURCES: &[&str] = &[
    "svelte",
    "vue",
    "astro",
    "html",
    "htm",
    "css",
    "scss",
    "sass",
    "less",
    "erb",
    "hbs",
    "handlebars",
    "mustache",
    "ejs",
    "njk",
    "jinja",
    "j2",
    "liquid",
    "twig",
    "php",
    "rb",
    "kt",
    "kts",
    "scala",
    "cs",
    "fs",
    "lua",
    "dart",
    "ex",
    "exs",
    "erl",
    "hs",
    "ml",
    "clj",
    "sql",
    "graphql",
    "gql",
    "proto",
    "sh",
    "bash",
    "zsh",
    "gd",
    "m",
    "mm",
    "groovy",
    "r",
    "jl",
    "nim",
    "cr",
    "elm",
    "purs",
    "sol",
    "tf",
    "nix",
];
/// Files bigger than this are generated, not written.
const MAX_BYTES: u64 = 512 * 1024;
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

/// Whether a file the outline can't read is a source of the project's
/// own, by its extension.
fn is_other_source(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| OTHER_SOURCES.contains(&ext.to_ascii_lowercase().as_str()))
}

/// What the files under `root` hold, relative to it.
#[derive(Default)]
struct Found {
    /// Files the outline reads, with their stamps.
    sources: Vec<(PathBuf, Option<Stamp>)>,
    /// `tsconfig*.json` files, for path aliases.
    configs: Vec<PathBuf>,
    /// Files it doesn't, placed by their folder.
    others: Vec<PathBuf>,
    packages: Vec<PathBuf>,
    readmes: HashMap<PathBuf, String>,
    skipped: usize,
}

/// Files that make their folder a package or a service.
const MANIFESTS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "go.mod",
    "pom.xml",
    "build.gradle",
    "build.gradle.kts",
    "pyproject.toml",
    "Package.swift",
    "build.zig",
];

/// What a README says its folder is for: its first paragraph's first
/// sentence, past its title and badges.
fn readme_purpose(text: &str) -> Option<String> {
    let paragraph: Vec<&str> = text
        .lines()
        .map(str::trim)
        .skip_while(|line| {
            line.is_empty()
                || line.starts_with('#')
                || line.starts_with('[')
                || line.starts_with('!')
                || line.starts_with('<')
                || line.starts_with("===")
                || line.starts_with("---")
        })
        .take_while(|line| !line.is_empty())
        .collect();
    let text = paragraph.join(" ");
    let end = text
        .match_indices(". ")
        .map(|(at, _)| at + 1)
        .find(|&at| at > 12)
        .unwrap_or(text.len());
    let purpose: String = text[..end].replace(['*', '`', '_'], "");
    (!purpose.is_empty()).then_some(purpose)
}

/// The source files under `root`, relative to it.
fn source_files(root: &Path) -> Found {
    let mut found = Vec::new();
    let mut configs = Vec::new();
    let mut others = Vec::new();
    let mut packages = Vec::new();
    let mut readmes = HashMap::new();
    let mut skipped = 0;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries.into_iter().rev() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if !name.starts_with('.') && !SKIPPED.contains(&name.as_str()) {
                    stack.push(path);
                }
            } else if kind.is_file() && MANIFESTS.contains(&name.as_str()) {
                if let Ok(relative) = dir.strip_prefix(root)
                    && !packages.contains(&relative.to_path_buf())
                {
                    packages.push(relative.to_path_buf());
                }
            } else if kind.is_file()
                && name.to_lowercase().starts_with("readme")
                && let Ok(relative) = dir.strip_prefix(root)
            {
                if let Some(purpose) = std::fs::read_to_string(&path)
                    .ok()
                    .and_then(|text| readme_purpose(&text))
                {
                    readmes.insert(relative.to_path_buf(), purpose);
                }
            } else if kind.is_file()
                && name.starts_with("tsconfig")
                && name.ends_with(".json")
                && let Ok(relative) = path.strip_prefix(root)
            {
                configs.push(relative.to_path_buf());
            } else if kind.is_file() && lang_of(&path).is_some() {
                let Ok(metadata) = entry.metadata() else {
                    continue;
                };
                if metadata.len() > MAX_BYTES {
                    continue;
                }
                let stamp = metadata.modified().ok().map(|at| (at, metadata.len()));
                if found.len() >= MAX_FILES {
                    skipped += 1;
                } else if let Ok(relative) = path.strip_prefix(root) {
                    found.push((relative.to_path_buf(), stamp));
                }
            } else if kind.is_file()
                && is_other_source(&path)
                && others.len() < MAX_FILES
                && let Ok(relative) = path.strip_prefix(root)
            {
                others.push(relative.to_path_buf());
            }
        }
    }
    found.sort();
    configs.sort();
    others.sort();
    packages.sort();
    Found {
        sources: found,
        configs,
        others,
        packages,
        readmes,
        skipped,
    }
}

impl Project {
    /// Read the project at `root`.
    #[cfg(test)]
    pub fn read(root: &Path) -> Project {
        Project::read_again(root, None)
    }

    /// Read the project at `root` again, keeping what `before` read of
    /// each file unchanged since.
    pub fn read_again(root: &Path, before: Option<&Project>) -> Project {
        let found = source_files(root);
        let kept: HashMap<&Path, usize> = before
            .filter(|before| before.root == root)
            .map(|before| {
                before
                    .files
                    .iter()
                    .enumerate()
                    .map(|(ix, file)| (file.path.as_path(), ix))
                    .collect()
            })
            .unwrap_or_default();
        let unchanged = |(path, stamp): &(PathBuf, Option<Stamp>)| {
            let ix = kept.get(path.as_path())?;
            let before = before?;
            (stamp.is_some() && before.files[*ix].stamp == *stamp).then_some(*ix)
        };
        let changed: Vec<PathBuf> = found
            .sources
            .iter()
            .filter(|source| unchanged(source).is_none())
            .map(|(path, _)| path.clone())
            .collect();
        let mut read = read_all(root, &changed).into_iter();
        let files = found
            .sources
            .iter()
            .map(|source| {
                let outline = match (unchanged(source), before) {
                    (Some(ix), Some(before)) => before.outline_of(ix),
                    _ => read.next().unwrap_or_default(),
                };
                (source.0.clone(), source.1, outline)
            })
            .collect();
        let aliases = aliases(root, &found.configs);
        let mut project = Project::from_stamped(root, files, aliases);
        project.others = found.others;
        project.packages = found.packages;
        project.readmes = found.readmes;
        project.skipped = found.skipped;
        project
    }

    /// File `file` as it read: its outline, functions and all.
    fn outline_of(&self, file: usize) -> Outline {
        let source = &self.files[file];
        Outline {
            purpose: source.purpose.clone(),
            package: source.package.clone(),
            imports: source.imports.clone(),
            types: source.types.clone(),
            functions: self.modules[source.module]
                .functions
                .iter()
                .map(|&id| &self.functions[id])
                .filter(|function| function.file == file)
                .map(|function| function.outline.clone())
                .collect(),
            variables: source.variables.clone(),
        }
    }

    /// The project made of these files, their stamps, and what they read
    /// as.
    fn from_stamped(
        root: &Path,
        files: Vec<(PathBuf, Option<Stamp>, Outline)>,
        aliases: Vec<Alias>,
    ) -> Project {
        let mut project = Project {
            root: root.to_path_buf(),
            aliases,
            ..Project::default()
        };
        let mut dirs: HashMap<PathBuf, usize> = HashMap::new();
        for (path, stamp, outline) in files {
            let Some(lang) = lang_of(&path) else {
                continue;
            };
            let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
            let module = *dirs.entry(dir.clone()).or_insert_with(|| {
                project.modules.push(Module {
                    dir: dir.clone(),
                    label: String::new(),
                    files: Vec::new(),
                    functions: Vec::new(),
                });
                project.modules.len() - 1
            });
            let file = project.files.len();
            project.modules[module].files.push(file);
            for function in outline.functions {
                project.modules[module]
                    .functions
                    .push(project.functions.len());
                project.functions.push(Func {
                    file,
                    outline: function,
                });
            }
            project.files.push(SourceFile {
                path,
                lang,
                purpose: outline.purpose,
                stamp,
                module,
                package: outline.package,
                imports: outline.imports,
                types: outline.types,
                variables: outline.variables,
            });
        }
        project.label_modules();
        project.resolve();
        project.module_imports();
        project
    }

    /// Modules are shown by package past the part every package shares, or
    /// by folder.
    fn label_modules(&mut self) {
        let packages: Vec<&str> = self
            .files
            .iter()
            .filter_map(|file| file.package.as_deref())
            .collect();
        let shared = packages.iter().skip(1).fold(
            packages
                .first()
                .map(|p| p.split('.').collect::<Vec<_>>())
                .unwrap_or_default(),
            |shared, package| {
                shared
                    .into_iter()
                    .zip(package.split('.'))
                    .take_while(|(a, b)| a == b)
                    .map(|(a, _)| a)
                    .collect()
            },
        );
        // Keep one part of what's shared, so the top package has a name.
        let keep = shared.len().saturating_sub(1);
        for module in &mut self.modules {
            let package = module
                .files
                .first()
                .and_then(|&file| self.files[file].package.as_deref());
            module.label = match package {
                Some(package) => package.split('.').skip(keep).collect::<Vec<_>>().join("."),
                None if module.dir.as_os_str().is_empty() => "(top)".into(),
                None => module.dir.to_string_lossy().replace('\\', "/"),
            };
        }
    }

    /// The functions named `short`.
    fn named(&self, short: &str) -> Vec<FnId> {
        self.by_short.get(short).cloned().unwrap_or_default()
    }

    fn resolve(&mut self) {
        let mut by_short: HashMap<String, Vec<FnId>> = HashMap::new();
        for (id, function) in self.functions.iter().enumerate() {
            by_short
                .entry(function.short().to_string())
                .or_default()
                .push(id);
        }
        self.by_short = by_short;
        let mut types: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        for (file, source) in self.files.iter().enumerate() {
            for (ix, ty) in source.types.iter().enumerate() {
                types.entry(ty.name.clone()).or_default().push((file, ix));
            }
        }
        self.types = types;
        let edges: Vec<Vec<Edge>> = (0..self.functions.len())
            .map(|id| {
                let function = &self.functions[id];
                function
                    .outline
                    .calls
                    .iter()
                    .enumerate()
                    .filter_map(|(call, site)| {
                        let target = if site.through_parameter {
                            Target::Unknown
                        } else {
                            self.target(id, site)?
                        };
                        if site.reference && !matches!(target, Target::Function(_)) {
                            return None;
                        }
                        Some(Edge {
                            from: id,
                            target,
                            call,
                            callback: site.reference || site.through_parameter,
                        })
                    })
                    .collect()
            })
            .collect();
        let mut callers = vec![Vec::new(); self.functions.len()];
        for edge in edges.iter().flatten() {
            if let Target::Function(to) = edge.target
                && !callers[to].contains(&edge.from)
                && to != edge.from
            {
                callers[to].push(edge.from);
            }
        }
        self.edges = edges;
        self.callers = callers;
    }

    /// What call `site` in function `from` reaches; `None` for calls to
    /// the language's own (built-in) functions, or names matching too
    /// much.
    fn target(&self, from: FnId, site: &Call) -> Option<Target> {
        let function = &self.functions[from];
        let file = &self.files[function.file];
        let named = self.named(&site.name);
        let owner = function.outline.owner.as_deref();
        match site.qualifier.as_deref() {
            // On `this`, or nothing: the function's own type and what it
            // extends, the same file, then what the file imports.
            None => {
                if let Some(owner) = owner
                    && let Some(found) = self.method_of(owner, &site.name)
                {
                    return Some(found);
                }
                let here: Vec<FnId> = named
                    .iter()
                    .copied()
                    .filter(|&id| self.functions[id].file == function.file)
                    .collect();
                if let [one] = here.as_slice() {
                    return Some(Target::Function(*one));
                }
                if let Some(found) = self.imported(function.file, &site.name, &site.name) {
                    return Some(found);
                }
                self.by_name(&named)
            }
            // On an expression with no name (`builder().build()`): a guess
            // by name would mostly be wrong, so none.
            Some("") => None,
            Some(qualifier) => {
                let ty = self.type_of(from, qualifier);
                if let Some(ty) = &ty {
                    if let Some(found) = self.method_of(ty, &site.name) {
                        return Some(found);
                    }
                    if self.types.contains_key(ty.as_str()) {
                        // A project type without it: inherited from outside.
                        return None;
                    }
                    if let Some(package) = self.package_of(function.file, ty) {
                        return Some(Target::External(package));
                    }
                }
                // A namespace or module imported by that name.
                if let Some(found) = self.imported(function.file, qualifier, &site.name) {
                    return Some(found);
                }
                // A typed name of the language's own, such as a string, or a
                // Java name the index can't type: not drawn.
                if ty.is_some() || file.lang == Lang::Java {
                    return None;
                }
                match named.len() {
                    1..=MAX_AMBIGUOUS => Some(Target::Ambiguous(named)),
                    _ => None,
                }
            }
        }
    }

    /// One function by name alone: certain if there's one, ambiguous if
    /// there are a few.
    fn by_name(&self, named: &[FnId]) -> Option<Target> {
        match named.len() {
            1 => Some(Target::Function(named[0])),
            2..=MAX_AMBIGUOUS => Some(Target::Ambiguous(named.to_vec())),
            _ => None,
        }
    }

    /// Method `name` of type `ty` or what it extends, in the project.
    fn method_of(&self, ty: &str, name: &str) -> Option<Target> {
        let mut seen = HashSet::new();
        let mut types = vec![ty.to_string()];
        while let Some(ty) = types.pop() {
            if !seen.insert(ty.clone()) {
                continue;
            }
            let found: Vec<FnId> = self
                .named(name)
                .into_iter()
                .filter(|&id| self.functions[id].outline.owner.as_deref() == Some(ty.as_str()))
                .collect();
            match found.as_slice() {
                [] => {}
                [one] => return Some(Target::Function(*one)),
                // Overloads: the first, which the server tells apart.
                [first, ..] => return Some(Target::Function(*first)),
            }
            for &(file, ix) in self.types.get(ty.as_str()).into_iter().flatten() {
                types.extend(self.files[file].types[ix].implements.iter().cloned());
            }
        }
        None
    }

    /// The type `name` is declared with in function `from`: a parameter,
    /// a local, a field of its type; or a type itself, by its capital.
    fn type_of(&self, from: FnId, name: &str) -> Option<String> {
        let function = &self.functions[from];
        if let Some((_, ty)) = function
            .outline
            .locals
            .iter()
            .rev()
            .find(|(n, _)| n == name)
        {
            return Some(ty.clone());
        }
        let file = &self.files[function.file];
        if let Some(owner) = function.outline.owner.as_deref()
            && let Some(ty) = file.types.iter().find(|t| t.name == owner)
            && let Some((_, field)) = ty.fields.iter().find(|(n, _)| n == name)
        {
            return Some(field.clone());
        }
        name.starts_with(char::is_uppercase)
            .then(|| name.to_string())
    }

    /// The package outside the project that file `file` gets type `ty`
    /// from.
    fn package_of(&self, file: usize, ty: &str) -> Option<String> {
        let source = &self.files[file];
        for import in &source.imports {
            if import.names.iter().any(|(local, _)| local == ty) {
                return match source.lang {
                    // A type of the project's own the index can't see
                    // (generated, say) is no package outside.
                    Lang::Java => match self.java_import(&import.path) {
                        JavaImport::Ours { .. } => None,
                        JavaImport::Theirs(package) => Some(package),
                    },
                    _ if import.path.starts_with('.') => None,
                    _ if self.resolve_import(file, &import.path).is_some() => None,
                    _ => Some(npm_package(&import.path)),
                };
            }
        }
        match source.lang {
            Lang::Java => Some("java.lang".into()),
            _ => None,
        }
    }

    /// What `name` (used as `called` is called on it, or called itself)
    /// is, when file `file` imports it: a function in another file of the
    /// project, or a package outside.
    fn imported(&self, file: usize, name: &str, called: &str) -> Option<Target> {
        let source = &self.files[file];
        let import = source
            .imports
            .iter()
            .find(|import| import.names.iter().any(|(local, _)| local == name))?;
        let exported = import
            .names
            .iter()
            .find(|(local, _)| local == name)
            .map(|(_, exported)| exported.as_str())
            .unwrap_or(name);
        match source.lang {
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => {
                let Some(target) = self.resolve_import(file, &import.path) else {
                    if import.path.starts_with('.') {
                        return None;
                    }
                    return Some(Target::External(npm_package(&import.path)));
                };
                // `fn()` imported as itself, or `ns.fn()` from a namespace.
                let wanted = if name == called { exported } else { called };
                let found: Vec<FnId> = self
                    .named(wanted)
                    .into_iter()
                    .filter(|&id| {
                        self.functions[id].file == target
                            || self.functions[id].outline.owner.as_deref() == Some(name)
                    })
                    .collect();
                self.by_name(&found)
            }
            Lang::Java => match self.java_import(&import.path) {
                // `import static a.b.Type.method`: the type's method.
                JavaImport::Ours {
                    member_of: Some(ty),
                } => self.method_of(&ty, called),
                JavaImport::Ours { member_of: None } => self.method_of(name, called),
                JavaImport::Theirs(package) => Some(Target::External(package)),
            },
            _ => None,
        }
    }

    /// Whether a Java import is of the project's own packages, and of a
    /// type's member (a static import), or which package outside.
    fn java_import(&self, path: &str) -> JavaImport {
        let ours = |package: &str| {
            self.files
                .iter()
                .any(|f| f.package.as_deref() == Some(package))
        };
        let Some((package, _)) = path.rsplit_once('.') else {
            return JavaImport::Theirs(path.to_string());
        };
        if ours(package) {
            return JavaImport::Ours { member_of: None };
        }
        if let Some((outer, ty)) = package.rsplit_once('.')
            && ours(outer)
        {
            return JavaImport::Ours {
                member_of: Some(ty.to_string()),
            };
        }
        JavaImport::Theirs(fold_package(package))
    }

    /// The project file an import from file `from` means: relative, or
    /// through a path alias; `None` for a package.
    pub fn resolve_import(&self, from: usize, path: &str) -> Option<usize> {
        if path.starts_with('.') {
            let dir = self.files[from].path.parent().unwrap_or(Path::new(""));
            return self.file_at(&normalize(&dir.join(path)));
        }
        let file = &self.files[from].path;
        let alias = self
            .aliases
            .iter()
            .filter(|alias| file.starts_with(&alias.dir) && path.starts_with(&alias.prefix))
            .max_by_key(|alias| alias.dir.components().count())?;
        self.file_at(&alias.target.join(&path[alias.prefix.len()..]))
    }

    /// The project file at `joined`, as imports name files: without their
    /// extension, `.js` for `.ts`, or a folder's index.
    fn file_at(&self, joined: &Path) -> Option<usize> {
        let stem = joined.to_string_lossy().into_owned();
        let stem = stem
            .strip_suffix(".js")
            .or_else(|| stem.strip_suffix(".mjs"))
            .unwrap_or(&stem)
            .to_string();
        let tries = [
            format!("{stem}.ts"),
            format!("{stem}.tsx"),
            format!("{stem}.js"),
            format!("{stem}/index.ts"),
            format!("{stem}/index.js"),
            stem.clone(),
        ];
        tries.iter().find_map(|candidate| {
            let candidate = Path::new(candidate);
            self.files.iter().position(|file| file.path == candidate)
        })
    }

    /// Which modules each module imports, from its files' imports.
    fn module_imports(&mut self) {
        let mut by_package: HashMap<&str, usize> = HashMap::new();
        for file in &self.files {
            if let Some(package) = file.package.as_deref() {
                by_package.insert(package, file.module);
            }
        }
        let mut imports: Vec<HashMap<usize, usize>> = vec![HashMap::new(); self.modules.len()];
        for (ix, file) in self.files.iter().enumerate() {
            for import in &file.imports {
                let target = match file.lang {
                    Lang::Java => {
                        let package = if import.wildcard {
                            import.path.as_str()
                        } else {
                            import.path.rsplit_once('.').map_or("", |(p, _)| p)
                        };
                        by_package.get(package).copied()
                    }
                    _ => self
                        .resolve_import(ix, &import.path)
                        .map(|file| self.files[file].module),
                };
                if let Some(target) = target.filter(|&target| target != file.module) {
                    *imports[file.module].entry(target).or_default() += 1;
                }
            }
        }
        // Java needs no import within a package, but across: the types a
        // file uses from other packages are imported, so this covers it.
        self.imports = imports
            .into_iter()
            .map(|map| {
                let mut list: Vec<(usize, usize)> = map.into_iter().collect();
                list.sort();
                list
            })
            .collect();
    }

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

enum JavaImport {
    Ours { member_of: Option<String> },
    Theirs(String),
}

/// A TypeScript path alias: under `dir`, imports starting `prefix` are
/// files under `target` (both relative to the project).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alias {
    pub dir: PathBuf,
    pub prefix: String,
    pub target: PathBuf,
}

/// The aliases in the `tsconfig*.json` files under `root`: each
/// `compilerOptions.paths` entry ending `/*`, from its `baseUrl`.
fn aliases(root: &Path, configs: &[PathBuf]) -> Vec<Alias> {
    let mut found = Vec::new();
    for config in configs {
        let Ok(text) = std::fs::read_to_string(root.join(config)) else {
            continue;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&without_comments(&text)) else {
            continue;
        };
        let options = &json["compilerOptions"];
        let dir = config.parent().map(Path::to_path_buf).unwrap_or_default();
        let base = normalize(&dir.join(options["baseUrl"].as_str().unwrap_or(".")));
        let Some(paths) = options["paths"].as_object() else {
            continue;
        };
        for (pattern, targets) in paths {
            let (Some(prefix), Some(target)) = (
                pattern.strip_suffix('*'),
                targets
                    .get(0)
                    .and_then(|t| t.as_str())
                    .and_then(|t| t.strip_suffix('*')),
            ) else {
                continue;
            };
            let alias = Alias {
                dir: dir.clone(),
                prefix: prefix.to_string(),
                target: normalize(&base.join(target)),
            };
            if !found.contains(&alias) {
                found.push(alias);
            }
        }
    }
    found
}

/// JSON with comments (as tsconfig files have them) without them, and
/// without trailing commas.
fn without_comments(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    while let Some(c) = chars.next() {
        if in_string {
            out.push(c);
            if c == '\\' {
                out.extend(chars.next());
            } else if c == '"' {
                in_string = false;
            }
            continue;
        }
        match (c, chars.peek()) {
            ('"', _) => {
                in_string = true;
                out.push(c);
            }
            ('/', Some('/')) => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            ('/', Some('*')) => {
                chars.next();
                let mut last = ' ';
                for c in chars.by_ref() {
                    if last == '*' && c == '/' {
                        break;
                    }
                    last = c;
                }
            }
            _ => out.push(c),
        }
    }
    // `,}` and `,]`.
    let mut cleaned = String::new();
    let bytes: Vec<char> = out.chars().collect();
    for (ix, &c) in bytes.iter().enumerate() {
        if c == ',' {
            let next = bytes[ix + 1..].iter().find(|c| !c.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                continue;
            }
        }
        cleaned.push(c);
    }
    cleaned
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

/// `org.springframework.data` from `org.springframework.data.jpa.repository`:
/// one node per library, not per package in it.
fn fold_package(package: &str) -> String {
    package.split('.').take(3).collect::<Vec<_>>().join(".")
}

/// `@scope/name` or `name` from an import of `@scope/name/sub`.
fn npm_package(path: &str) -> String {
    let mut parts = path.split('/');
    let first = parts.next().unwrap_or(path);
    if first.starts_with('@') {
        match parts.next() {
            Some(second) => format!("{first}/{second}"),
            None => first.to_string(),
        }
    } else {
        first.to_string()
    }
}

/// `a/b/../c` as `a/c`.
pub(crate) fn normalize(path: &Path) -> PathBuf {
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

/// Read every file, on as many threads as there are cores.
fn read_all(root: &Path, paths: &[PathBuf]) -> Vec<Outline> {
    let threads = std::thread::available_parallelism()
        .map_or(4, |n| n.get())
        .min(12);
    let chunk = paths.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = paths
            .chunks(chunk)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|path| {
                            let lang = lang_of(path).expect("listed by language");
                            std::fs::read_to_string(root.join(path))
                                .map(|source| outline::outline(&source, lang))
                                .unwrap_or_default()
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A small project: Java in two packages, TypeScript in two folders.
    pub(crate) fn fixture() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let files = [
            (
                "server/src/org/demo/bugs/BugService.java",
                "package org.demo.bugs;\n\nimport org.demo.users.UserService;\nimport org.slf4j.Logger;\n\npublic class BugService {\n    private final UserService users;\n    private final Logger log;\n    private final Notifier notifier;\n\n    public Bug create(String title, long userId) {\n        User user = users.find(userId);\n        Bug bug = save(title, user);\n        log.info(\"made\");\n        notifier.send(bug);\n        return bug;\n    }\n\n    private Bug save(String title, User user) {\n        return new Bug(title, user);\n    }\n}\n",
            ),
            (
                "server/src/org/demo/bugs/Notifier.java",
                "package org.demo.bugs;\n\npublic interface Notifier {\n    void send(Bug bug);\n}\n",
            ),
            (
                "server/src/org/demo/bugs/MailNotifier.java",
                "package org.demo.bugs;\n\npublic class MailNotifier implements Notifier {\n    public void send(Bug bug) {\n        format(bug);\n    }\n\n    private String format(Bug bug) { return \"\"; }\n}\n",
            ),
            (
                "server/src/org/demo/users/UserService.java",
                "package org.demo.users;\n\npublic class UserService {\n    public User find(long id) {\n        return lookup(id);\n    }\n\n    private User lookup(long id) { return null; }\n}\n",
            ),
            (
                "client/src/rooms.ts",
                "import { connect } from './net/socket'\nimport * as mediasoup from 'mediasoup'\n\nexport async function join(id: string, done: () => void): Promise<void> {\n    const socket = connect(id)\n    await mediasoup.createRouter()\n    send(socket, id)\n    done()\n}\n\nfunction send(socket: Socket, id: string) {}\n",
            ),
            (
                "client/src/net/socket.ts",
                "export function connect(url: string): Socket {\n    return open(url)\n}\n\nfunction open(url: string): Socket { return null as any }\n",
            ),
            (
                "client/node_modules/mediasoup/index.ts",
                "export function createRouter() {}\n",
            ),
        ];
        for (name, text) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        dir
    }

    pub(crate) fn id(project: &Project, title: &str) -> FnId {
        project
            .functions
            .iter()
            .position(|f| f.title() == title)
            .unwrap_or_else(|| panic!("no {title}"))
    }

    fn targets(project: &Project, title: &str) -> Vec<String> {
        project.edges[id(project, title)]
            .iter()
            .map(|edge| match &edge.target {
                Target::Function(to) => project.functions[*to].title(),
                Target::Ambiguous(ids) => format!("~{}", ids.len()),
                Target::External(package) => format!("ext {package}"),
                Target::Unknown => "?".into(),
            })
            .collect()
    }

    #[test]
    fn reads_modules_and_skips_dependencies() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let labels: Vec<&str> = project.modules.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(
            labels,
            ["client/src/net", "client/src", "demo.bugs", "demo.users"]
        );
        assert!(
            !project
                .files
                .iter()
                .any(|f| f.path.starts_with("client/node_modules"))
        );
    }

    #[test]
    fn java_calls_follow_types_imports_and_interfaces() {
        let dir = fixture();
        let project = Project::read(dir.path());
        assert_eq!(
            targets(&project, "BugService.create"),
            [
                "UserService.find",
                "BugService.save",
                "ext org.slf4j",
                "Notifier.send"
            ]
        );
        let send = id(&project, "Notifier.send");
        let implementations: Vec<String> = project
            .implementations(send)
            .into_iter()
            .map(|id| project.functions[id].title())
            .collect();
        assert_eq!(implementations, ["MailNotifier.send"]);
        let find = id(&project, "UserService.find");
        assert_eq!(project.callers[find], [id(&project, "BugService.create")]);
    }

    #[test]
    fn typescript_calls_follow_imports_and_parameters() {
        let dir = fixture();
        let project = Project::read(dir.path());
        assert_eq!(
            targets(&project, "join"),
            ["connect", "ext mediasoup", "send", "?"]
        );
        let rooms = project
            .modules
            .iter()
            .position(|m| m.label == "client/src")
            .unwrap();
        let net = project
            .modules
            .iter()
            .position(|m| m.label == "client/src/net")
            .unwrap();
        assert_eq!(project.imports[rooms], [(net, 1)]);
        let bugs = project
            .modules
            .iter()
            .position(|m| m.label == "demo.bugs")
            .unwrap();
        let users = project
            .modules
            .iter()
            .position(|m| m.label == "demo.users")
            .unwrap();
        assert_eq!(project.imports[bugs], [(users, 1)]);
    }

    #[test]
    fn symbols_are_found_again_after_a_rename() {
        let dir = fixture();
        let project = Project::read(dir.path());
        let mut symbol = project.symbol(id(&project, "UserService.find"));
        assert_eq!(
            project.find(&symbol),
            Some(id(&project, "UserService.find"))
        );
        symbol.file = "server/src/org/demo/users/Old.java".into();
        assert_eq!(
            project.find(&symbol),
            Some(id(&project, "UserService.find"))
        );
        symbol.name = "gone".into();
        assert_eq!(project.find(&symbol), None);
    }

    /// How a real project reads: `JIG_MAP_PROJECT=<folder>`, `--ignored
    /// --nocapture`. Only reads it.
    #[test]
    #[ignore]
    fn reads_a_real_project() {
        let root = std::env::var_os("JIG_MAP_PROJECT").expect("JIG_MAP_PROJECT");
        let started = std::time::Instant::now();
        let project = Project::read(Path::new(&root));
        let took = started.elapsed();
        let mut kinds = [0usize; 4];
        for edge in project.edges.iter().flatten() {
            kinds[match edge.target {
                Target::Function(_) => 0,
                Target::Ambiguous(_) => 1,
                Target::External(_) => 2,
                Target::Unknown => 3,
            }] += 1;
        }
        let hubs: Vec<String> = (0..project.functions.len())
            .filter(|&id| project.is_hub(id))
            .map(|id| project.functions[id].title())
            .collect();
        println!(
            "{} files, {} modules, {} functions in {took:?}; calls: {} certain, {} ambiguous, {} external, {} unknown; hubs: {hubs:?}",
            project.files.len(),
            project.modules.len(),
            project.functions.len(),
            kinds[0],
            kinds[1],
            kinds[2],
            kinds[3]
        );
        let mut externals: HashMap<&str, usize> = HashMap::new();
        for edge in project.edges.iter().flatten() {
            if let Target::External(package) = &edge.target {
                *externals.entry(package.as_str()).or_default() += 1;
            }
        }
        let mut externals: Vec<_> = externals.into_iter().collect();
        externals.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
        println!("externals: {:?}", &externals[..externals.len().min(25)]);
        let imports: usize = project.imports.iter().map(Vec::len).sum();
        println!("module imports: {imports}");
        for module in project.modules.iter().take(80) {
            println!("  {} ({} fns)", module.label, module.functions.len());
        }
    }
}
