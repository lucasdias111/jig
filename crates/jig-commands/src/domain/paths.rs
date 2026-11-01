//! Paths as the pickers show them: relative to the project, with `/`
//! separators.

use std::path::Path;

/// `path` relative to `root` with `/` separators, if it is inside it.
pub fn relative_path(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let parts: Vec<_> = rest
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect();
    (!parts.is_empty()).then(|| parts.join("/"))
}
