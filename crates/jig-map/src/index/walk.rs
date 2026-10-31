//! Finding the project's source files on disk, and the path aliases its configs set.

use super::*;

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

/// Whether a file the outline can't read is a source of the project's
/// own, by its extension.
fn is_other_source(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| OTHER_SOURCES.contains(&ext.to_ascii_lowercase().as_str()))
}

/// What the files under `root` hold, relative to it.
#[derive(Default)]
pub(super) struct Found {
    /// Files the outline reads, with their stamps.
    pub(super) sources: Vec<(PathBuf, Option<Stamp>)>,
    /// `tsconfig*.json` files, for path aliases.
    pub(super) configs: Vec<PathBuf>,
    /// Files it doesn't, placed by their folder.
    pub(super) others: Vec<PathBuf>,
    pub(super) packages: Vec<PathBuf>,
    pub(super) readmes: HashMap<PathBuf, String>,
    pub(super) skipped: usize,
}

/// Files that make their folder a package or a service.
pub(super) const MANIFESTS: &[&str] = &[
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
pub(super) fn source_files(root: &Path) -> Found {
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

/// The aliases in the `tsconfig*.json` files under `root`: each
/// `compilerOptions.paths` entry ending `/*`, from its `baseUrl`.
pub(super) fn aliases(root: &Path, configs: &[PathBuf]) -> Vec<Alias> {
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
