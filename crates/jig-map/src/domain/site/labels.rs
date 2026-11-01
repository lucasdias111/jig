//! What a closed box says: its purpose or what's inside, and how much.

use super::*;

/// Names in a line: the first few, and how many more.
pub(super) fn summary(names: &[String]) -> String {
    const SHOWN: usize = 4;
    let mut out = names
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ");
    if names.len() > SHOWN {
        out.push_str(&format!(" · +{}", names.len() - SHOWN));
    }
    out
}

/// What a folder's own module file says it's for: `mod.rs`, the
/// `name.rs` beside it, `__init__.py`, `index.ts`, `doc.go`,
/// `package-info.java`.
pub(super) fn folder_doc(project: &Project, dir: &Path) -> Option<String> {
    let name = dir.file_name()?.to_string_lossy().into_owned();
    let candidates = [
        dir.join("mod.rs"),
        dir.with_file_name(format!("{name}.rs")),
        dir.join("lib.rs"),
        dir.join("__init__.py"),
        dir.join("index.ts"),
        dir.join("index.js"),
        dir.join("doc.go"),
        dir.join("package-info.java"),
    ];
    candidates.iter().find_map(|path| {
        project
            .files
            .iter()
            .find(|file| &file.path == path)?
            .purpose
            .clone()
    })
}

pub(super) fn file_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| slashed(path), |name| name.to_string_lossy().into_owned())
}

pub(super) fn plural(count: usize, what: &str) -> String {
    format!("{count} {what}{}", if count == 1 { "" } else { "s" })
}
