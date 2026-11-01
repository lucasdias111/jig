//! How files link to one another, for a file the index can't read (a
//! Svelte or Vue component, a page, a stylesheet): which files import it
//! and which it imports. Its functions aren't read, but its import lines
//! are easy to find (`import … from '…'`, `@import`, `src="…"`), as are
//! the read files' imports, which the index already has. A file beside it
//! with the same name (`+page.server.js` and `+page.svelte`) is linked
//! too: frameworks pair them by name.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;

use super::index::{Project, normalize};
use super::plan::slashed;

/// The project's files as they are now, for what the index doesn't keep:
/// the text of the files it can't read. The store reads them from disk.
pub trait Sources {
    /// File `place`'s bytes, relative to the project; `None` when it can't
    /// be read.
    fn read(&self, place: &Path) -> Option<Vec<u8>>;
}

/// Files read for their import lines, at most.
const MAX_SCANNED: usize = 5000;

/// One file's links to another, summed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The other file, relative.
    pub path: String,
    /// How many import lines join them; 0 for a file paired by name.
    pub imports: usize,
}

impl Link {
    /// What joins them, in words.
    pub fn how(&self, into: bool) -> String {
        match (self.imports, into) {
            (0, _) => "beside it, same name".into(),
            (1, true) => "imports it".into(),
            (1, false) => "imported by it".into(),
            (n, true) => format!("imports it {n}×"),
            (n, false) => format!("imported by it {n}×"),
        }
    }
}

/// The files linking to file `place`, and those it links to; each list
/// by path.
pub fn links_of(project: &Project, sources: &dyn Sources, place: &str) -> (Vec<Link>, Vec<Link>) {
    let all: HashSet<PathBuf> = project
        .files
        .iter()
        .map(|file| file.path.clone())
        .chain(project.others.iter().cloned())
        .collect();
    let target = PathBuf::from(place);
    let mut into: BTreeMap<String, usize> = BTreeMap::new();
    let mut out: BTreeMap<String, usize> = BTreeMap::new();
    let mut count = |from: &Path, to: &Path| {
        if to == target && from != target {
            *into.entry(slashed(from)).or_default() += 1;
        } else if from == target && to != target {
            *out.entry(slashed(to)).or_default() += 1;
        }
    };
    for file in &project.files {
        for import in &file.imports {
            if let Some(to) = resolve(project, &all, &file.path, &import.path) {
                count(&file.path, &to);
            }
        }
    }
    for from in project.others.iter().take(MAX_SCANNED) {
        let Some(text) = sources
            .read(from)
            .and_then(|bytes| String::from_utf8(bytes).ok())
        else {
            continue;
        };
        for spec in import_lines(&text) {
            if let Some(to) = resolve(project, &all, from, &spec) {
                count(from, &to);
            }
        }
    }
    let links = |found: BTreeMap<String, usize>| -> Vec<Link> {
        found
            .into_iter()
            .map(|(path, imports)| Link { path, imports })
            .collect()
    };
    let mut into = links(into);
    // Beside it with the same name, linked by the framework.
    for sibling in siblings(&all, &target) {
        let path = slashed(&sibling);
        if !into.iter().any(|link| link.path == path) {
            into.push(Link { path, imports: 0 });
        }
    }
    (into, links(out))
}

/// What a file the index can't read is, by its extension.
pub fn kind_of(path: &str) -> String {
    let extension = path.rsplit_once('.').map_or("", |(_, ext)| ext);
    match extension.to_ascii_lowercase().as_str() {
        "svelte" => "Svelte component".into(),
        "vue" => "Vue component".into(),
        "astro" => "Astro page".into(),
        "html" | "htm" => "HTML page".into(),
        "css" | "scss" | "sass" | "less" => "Stylesheet".into(),
        "erb" | "hbs" | "handlebars" | "ejs" | "njk" | "twig" | "liquid" => "Template".into(),
        "" => "File".into(),
        other => format!("{} file", other.to_uppercase()),
    }
}

/// How big file `place` is: its lines and bytes.
pub fn size_of(sources: &dyn Sources, place: &str) -> Option<String> {
    let text = sources.read(Path::new(place))?;
    let lines = text.iter().filter(|&&b| b == b'\n').count().max(1);
    let size = if text.len() >= 1024 {
        format!("{:.1} KB", text.len() as f32 / 1024.)
    } else {
        format!("{} bytes", text.len())
    };
    Some(format!(
        "{lines} line{} · {size}",
        if lines == 1 { "" } else { "s" }
    ))
}

/// The paths a file's import lines name, as written.
fn import_lines(text: &str) -> Vec<String> {
    static PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
            r#"(?:import|export)\s[^'";]*?\bfrom\s*['"]([^'"]+)['"]"#,
            r#"\bimport\s*\(?\s*['"]([^'"]+)['"]"#,
            r#"@(?:import|use|forward)\s+(?:url\()?\s*['"]([^'"]+)['"]"#,
            r#"<(?:script|link|img|source|iframe)\b[^>]*?\b(?:src|href)\s*=\s*['"]([^'"]+)['"]"#,
        ]
        .iter()
        .map(|pattern| Regex::new(pattern).expect("a pattern"))
        .collect()
    });
    let mut found = Vec::new();
    for pattern in PATTERNS.iter() {
        for capture in pattern.captures_iter(text) {
            found.push(capture[1].to_string());
        }
    }
    found
}

/// The project file an import of `spec` from `from` means: relative,
/// through a path alias, or SvelteKit's `$lib`; tried as written, then
/// with the extensions imports leave out.
fn resolve(project: &Project, all: &HashSet<PathBuf>, from: &Path, spec: &str) -> Option<PathBuf> {
    let spec = spec.split(['?', '#']).next().unwrap_or(spec);
    let dir = from.parent().unwrap_or(Path::new(""));
    let base = if spec.starts_with('.') {
        normalize(&dir.join(spec))
    } else if let Some(rest) = spec.strip_prefix("$lib/") {
        let lib = from
            .ancestors()
            .skip(1)
            .map(|at| at.join("src/lib"))
            .find(|lib| all.iter().any(|file| file.starts_with(lib)))?;
        lib.join(rest)
    } else if let Some(alias) = project
        .aliases
        .iter()
        .filter(|alias| from.starts_with(&alias.dir) && spec.starts_with(&alias.prefix))
        .max_by_key(|alias| alias.dir.components().count())
    {
        normalize(&alias.target.join(&spec[alias.prefix.len()..]))
    } else {
        // From the site's root: tried from each folder up; else a package.
        let rest = spec.strip_prefix('/')?;
        return from
            .ancestors()
            .skip(1)
            .map(|at| at.join(rest))
            .find(|path| all.contains(path));
    };
    let stem = base.to_string_lossy().into_owned();
    let stem = stem.strip_suffix(".js").unwrap_or(&stem).to_string();
    [
        base.to_string_lossy().into_owned(),
        format!("{stem}.ts"),
        format!("{stem}.js"),
        format!("{stem}.tsx"),
        format!("{stem}.jsx"),
        format!("{stem}.svelte"),
        format!("{stem}.vue"),
        format!("{stem}/index.ts"),
        format!("{stem}/index.js"),
    ]
    .into_iter()
    .map(PathBuf::from)
    .find(|path| all.contains(path))
}

/// The files beside `path` sharing its name up to the first dot.
fn siblings(all: &HashSet<PathBuf>, path: &Path) -> Vec<PathBuf> {
    let stem = |path: &Path| {
        path.file_name().map(|name| {
            name.to_string_lossy()
                .split('.')
                .next()
                .unwrap_or("")
                .to_string()
        })
    };
    let Some(name) = stem(path).filter(|name| !name.is_empty()) else {
        return Vec::new();
    };
    let mut found: Vec<PathBuf> = all
        .iter()
        .filter(|other| {
            *other != path && other.parent() == path.parent() && stem(other).as_ref() == Some(&name)
        })
        .cloned()
        .collect();
    found.sort();
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::cache::Folder;

    fn project() -> (tempfile::TempDir, Project) {
        let dir = tempfile::tempdir().unwrap();
        let write = |name: &str, text: &str| {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        };
        write("web/package.json", "{}\n");
        write(
            "web/src/lib/Form.svelte",
            "<script>\n  import { send } from './api';\n</script>\n<form></form>\n",
        );
        write("web/src/lib/api.ts", "export function send() {}\n");
        write(
            "web/src/routes/+page.svelte",
            "<script>\n  import Form from '$lib/Form.svelte';\n  import Other from '$lib/Form.svelte';\n</script>\n",
        );
        write(
            "web/src/routes/+page.server.js",
            "export function load() { return {} }\n",
        );
        write(
            "web/src/main.ts",
            "import Form from './lib/Form.svelte'\nexport function start() {}\n",
        );
        let project = Project::read(dir.path());
        (dir, project)
    }

    #[test]
    fn a_component_is_linked_from_what_imports_it_and_to_what_it_imports() {
        let (dir, project) = project();
        let (into, out) = links_of(&project, &Folder(dir.path()), "web/src/lib/Form.svelte");
        assert_eq!(
            into,
            [
                Link {
                    path: "web/src/main.ts".into(),
                    imports: 1
                },
                Link {
                    path: "web/src/routes/+page.svelte".into(),
                    imports: 2
                },
            ]
        );
        assert_eq!(
            out,
            [Link {
                path: "web/src/lib/api.ts".into(),
                imports: 1
            }]
        );
    }

    #[test]
    fn a_page_is_linked_from_the_file_beside_it_with_its_name() {
        let (dir, project) = project();
        let (into, _) = links_of(&project, &Folder(dir.path()), "web/src/routes/+page.svelte");
        assert_eq!(
            into,
            [Link {
                path: "web/src/routes/+page.server.js".into(),
                imports: 0
            }]
        );
        assert_eq!(into[0].how(true), "beside it, same name");
    }

    #[test]
    fn what_a_file_is_and_how_big() {
        let (dir, _) = project();
        assert_eq!(kind_of("a/B.svelte"), "Svelte component");
        assert_eq!(kind_of("a/b.scss"), "Stylesheet");
        assert_eq!(
            size_of(&Folder(dir.path()), "web/src/lib/api.ts").as_deref(),
            Some("1 line · 26 bytes")
        );
    }
}
