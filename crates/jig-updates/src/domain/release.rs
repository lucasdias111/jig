//! The latest release on GitHub: which one it is, its build for this
//! computer, and proof that the build is ours.

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Deserialize;

/// Where releases are published.
pub const REPOSITORY: &str = "lucasdias111/jig";

/// The key `.github/workflows/release.yml` signs every build with
/// (`minisign`, its secret key in the `MINISIGN_KEY` secret). A download
/// whose signature doesn't check out against it is never installed.
const PUBLIC_KEY: &str = "RWTZhCw4CZYRJ1vaMaA+y01yk6jm9xdoUMcoQ/3KxQ8KVO/SBITQH2ee";

/// The page people download Jig from, for when it can't update itself.
pub fn releases_page() -> String {
    format!("https://github.com/{REPOSITORY}/releases/latest")
}

/// A version as `major.minor.patch`, compared field by field.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u64, pub u64, pub u64);

impl Version {
    /// `0.3.0` or `v0.3.0`. Anything else, such as `0.3.0-beta`, is `None`,
    /// so a pre-release is never offered.
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().trim_start_matches('v').split('.');
        let mut next = || parts.next()?.parse().ok();
        let version = Version(next()?, next()?, next()?);
        parts.next().is_none().then_some(version)
    }

    /// The version running.
    pub fn current() -> Self {
        Self::parse(env!("CARGO_PKG_VERSION")).expect("the crate's version is major.minor.patch")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

/// The build of a release for this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub asset: String,
    pub url: String,
    pub signature_url: String,
}

/// The name of this computer's build, as the release workflow names it.
/// `None` where no build is published.
pub fn asset_name() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some("Jig-macos-arm64.zip"),
        ("linux", "x86_64") => Some("Jig-linux-x86_64.tar.gz"),
        ("windows", "x86_64") => Some("Jig-windows-x86_64.zip"),
        _ => None,
    }
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    name: String,
    browser_download_url: String,
}

/// Read GitHub's answer for `asset`, newer than `current`.
pub fn pick(json: &str, asset: &str, current: Version) -> Result<Option<Release>> {
    let release: GitHubRelease =
        serde_json::from_str(json).context("GitHub's answer didn't make sense")?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let Some(version) = Version::parse(&release.tag_name).filter(|version| *version > current)
    else {
        return Ok(None);
    };
    let url = |name: &str| {
        release
            .assets
            .iter()
            .find(|candidate| candidate.name == name)
            .map(|found| found.browser_download_url.clone())
    };
    // A release still uploading, or from before builds were signed.
    let (Some(url), Some(signature_url)) = (url(asset), url(&format!("{asset}.minisig"))) else {
        return Ok(None);
    };
    Ok(Some(Release {
        version,
        asset: asset.to_string(),
        url,
        signature_url,
    }))
}

/// Check that `archive` was signed with Jig's key, and signed as this
/// release's build: the signature's trusted comment, which the key also
/// signs, says `jig <version> <asset>`. That keeps an older signed build
/// from being passed off as the latest.
pub fn verify(archive: &[u8], signature: &str, release: &Release) -> Result<()> {
    verify_with(PUBLIC_KEY, archive, signature, release)
}

fn verify_with(key: &str, archive: &[u8], signature: &str, release: &Release) -> Result<()> {
    let key = minisign_verify::PublicKey::from_base64(key).context("Jig's update key")?;
    let signature = minisign_verify::Signature::decode(signature)
        .map_err(|error| anyhow!("The update's signature couldn't be read: {error}"))?;
    key.verify(archive, &signature, false)
        .map_err(|_| anyhow!("The update isn't signed by Jig's key, so it wasn't installed."))?;
    let expected = format!("jig {} {}", release.version, release.asset);
    if signature.trusted_comment().trim() != expected {
        bail!(
            "The update is signed as “{}”, not as {expected}, so it wasn't installed.",
            signature.trusted_comment().trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_by_number() {
        assert_eq!(Version::parse("v0.10.0"), Some(Version(0, 10, 0)));
        assert!(Version::parse("0.10.0") > Version::parse("0.9.3"));
        assert_eq!(Version::parse("0.3.0-beta"), None);
        assert_eq!(Version::parse("0.3"), None);
        assert_eq!(Version::parse("1.2.3.4"), None);
        assert_eq!(Version::current().to_string(), env!("CARGO_PKG_VERSION"));
    }

    fn answer(tag: &str, assets: &[&str]) -> String {
        let assets: Vec<String> = assets
            .iter()
            .map(|name| {
                format!(r#"{{"name": "{name}", "browser_download_url": "https://dl/{name}"}}"#)
            })
            .collect();
        format!(
            r#"{{"tag_name": "{tag}", "draft": false, "prerelease": false, "assets": [{}]}}"#,
            assets.join(",")
        )
    }

    #[test]
    fn picks_a_newer_signed_build_for_this_computer() {
        let asset = "Jig-linux-x86_64.tar.gz";
        let signed = [
            asset,
            "Jig-linux-x86_64.tar.gz.minisig",
            "Jig-macos-arm64.zip",
        ];
        let current = Version(0, 2, 0);

        let release = pick(&answer("v0.3.0", &signed), asset, current)
            .unwrap()
            .unwrap();
        assert_eq!(release.version, Version(0, 3, 0));
        assert_eq!(release.url, "https://dl/Jig-linux-x86_64.tar.gz");
        assert_eq!(
            release.signature_url,
            "https://dl/Jig-linux-x86_64.tar.gz.minisig"
        );

        assert_eq!(
            pick(&answer("v0.2.0", &signed), asset, current).unwrap(),
            None
        );
        assert_eq!(
            pick(&answer("v0.1.9", &signed), asset, current).unwrap(),
            None
        );
        assert_eq!(
            pick(&answer("v0.3.0", &[asset]), asset, current).unwrap(),
            None,
            "unsigned"
        );
        assert_eq!(
            pick(
                &answer("v0.3.0", &signed),
                "Jig-windows-x86_64.zip",
                current
            )
            .unwrap(),
            None,
            "no build for it"
        );
        assert!(pick("<html>", asset, current).is_err());
    }

    /// Asks GitHub for real: `cargo test -p jig-app latest_release_live -- --ignored`.
    // Made with `minisign -G -W` and
    // `minisign -S -m archive -t "jig 0.3.0 Jig-linux-x86_64.tar.gz"`
    // over the bytes `jig update test\n`.
    const TEST_KEY: &str = "RWSzsQ8CscRRpvaydFHBC/bjhQfoRbtWwzNdQD8A/PWSHoObwAkEt24a";
    const TEST_ARCHIVE: &[u8] = b"jig update test\n";

    fn release(version: Version) -> Release {
        Release {
            version,
            asset: "Jig-linux-x86_64.tar.gz".into(),
            url: String::new(),
            signature_url: String::new(),
        }
    }

    #[test]
    fn only_a_build_signed_as_this_release_checks_out() {
        let signature = include_str!("test.minisig");
        let release = release(Version(0, 3, 0));
        verify_with(TEST_KEY, TEST_ARCHIVE, signature, &release).unwrap();

        assert!(verify_with(TEST_KEY, b"something else", signature, &release).is_err());
        assert!(
            verify_with(PUBLIC_KEY, TEST_ARCHIVE, signature, &release).is_err(),
            "another key"
        );
        let error = verify_with(
            TEST_KEY,
            TEST_ARCHIVE,
            signature,
            &self::release(Version(0, 4, 0)),
        )
        .unwrap_err();
        assert!(error.to_string().contains("signed as"), "{error}");
    }
}
