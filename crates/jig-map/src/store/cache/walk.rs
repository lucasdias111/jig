//! Finding the project's source files on disk, and the path aliases its
//! configs set.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::domain::index::configs::{aliases_in, readme_purpose};
use crate::domain::index::{Alias, Stamp, lang_of};

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
    /// How long each of those is.
    pub(super) other_lengths: HashMap<PathBuf, u64>,
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

/// The source files under `root`, relative to it.
pub(super) fn source_files(root: &Path) -> Found {
    let mut found = Vec::new();
    let mut configs = Vec::new();
    let mut others = Vec::new();
    let mut other_lengths = HashMap::new();
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
                if let Ok(metadata) = entry.metadata() {
                    other_lengths.insert(relative.to_path_buf(), metadata.len());
                }
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
        other_lengths,
        packages,
        readmes,
        skipped,
    }
}

/// The aliases in the `tsconfig*.json` files under `root`.
pub(super) fn aliases(root: &Path, configs: &[PathBuf]) -> Vec<Alias> {
    let mut found = Vec::new();
    for config in configs {
        let Ok(text) = std::fs::read_to_string(root.join(config)) else {
            continue;
        };
        for alias in aliases_in(config, &text) {
            if !found.contains(&alias) {
                found.push(alias);
            }
        }
    }
    found
}
