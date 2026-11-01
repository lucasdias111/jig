//! What the Map saves in the project's `.jig/map/`: the domains, the
//! user's sketches, pinned drawings and kept purposes (committed), and how
//! each view is arranged (not). Also the images Export Image saves.

pub mod domains;
pub mod pins;
pub mod places;
pub mod purposes;
pub mod sketches;

use std::path::Path;

use anyhow::{Context as _, Result};

/// Write an exported image to `path`, where the user asked.
pub fn write_image(path: &Path, bytes: impl AsRef<[u8]>) -> Result<()> {
    std::fs::write(path, bytes).context("writing the image")
}
