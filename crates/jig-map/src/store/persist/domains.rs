//! The domains as saved in `.jig/map/domains.toml`, committed.

use std::path::Path;

use anyhow::{Context as _, Result};

use crate::domain::domains::{Domains, FILE};

impl Domains {
    /// The project's saved domains, if Index Domains has been kept.
    pub fn read(root: &Path) -> Option<Domains> {
        let text = std::fs::read_to_string(root.join(FILE)).ok()?;
        toml::from_str(&text).ok()
    }

    /// Save them for the project, in the format `read` reads.
    pub fn write(&self, root: &Path) -> Result<()> {
        let path = root.join(FILE);
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        std::fs::write(&path, self.to_toml()).with_context(|| format!("writing {}", path.display()))
    }
}

/// No domains saved: the file removed, if there was one.
pub fn remove(root: &Path) -> Result<()> {
    std::fs::remove_file(root.join(FILE)).or(Ok(()))
}
