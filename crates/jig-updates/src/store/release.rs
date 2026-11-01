//! Asking GitHub for the latest release and downloading its build.

use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};

use crate::domain::release::{REPOSITORY, Release, Version, asset_name, pick, verify};

/// Where downloaded updates wait for a restart.
pub fn download_dir() -> PathBuf {
    std::env::temp_dir().join("jig-update")
}

/// Drop a downloaded update once it's installed.
pub fn discard(archive: &Path) {
    let _ = fs::remove_file(archive);
}

fn agent(timeout: Duration) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .user_agent(concat!("jig/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// The latest release, if it's newer than `current` and has a build for
/// this computer.
pub fn newer_than(current: Version) -> Result<Option<Release>> {
    let url = format!("https://api.github.com/repos/{REPOSITORY}/releases/latest");
    let json = agent(Duration::from_secs(30))
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|error| anyhow!("Couldn't reach GitHub: {error}"))?
        .body_mut()
        .read_to_string()?;
    let Some(asset) = asset_name() else {
        return Ok(None);
    };
    pick(&json, asset, current)
}

/// Download `release` into `dir` and check its signature. Returns the
/// archive, which is only kept when it checks out.
pub fn download(release: &Release, dir: &Path) -> Result<PathBuf> {
    let agent = agent(Duration::from_secs(15 * 60));
    let fetch = |url: &str| -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        agent
            .get(url)
            .call()
            .map_err(|error| anyhow!("Couldn't download Jig {}: {error}", release.version))?
            .body_mut()
            .with_config()
            .limit(500 * 1024 * 1024)
            .reader()
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    };
    let signature = String::from_utf8(fetch(&release.signature_url)?)
        .context("The update's signature isn't text")?;
    let archive = fetch(&release.url)?;
    verify(&archive, &signature, release)?;
    fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = dir.join(&release.asset);
    fs::write(&path, archive).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore]
    fn latest_release_live() {
        // Older than any release, so the latest is offered once it's signed.
        let found = newer_than(Version(0, 0, 1)).unwrap();
        eprintln!("{found:?}");
    }
}
