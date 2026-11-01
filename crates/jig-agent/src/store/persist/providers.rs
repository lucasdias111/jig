//! The providers on disk and on the network: `config.toml`, the keys saved
//! from Settings, and asking a provider for its models.

use std::path::{Path, PathBuf};

use jig_ai::{Config, ConfigFile, KeyStore, ProviderConfig};

/// The user's `config.toml` and the system's key store.
pub fn user_files() -> (Option<PathBuf>, Option<KeyStore>) {
    (Config::user_path(), KeyStore::system())
}

/// `config.toml` and a keys file in `dir`, kept apart from the user's.
#[cfg(any(test, feature = "testing", feature = "snapshots"))]
pub fn files_in(dir: &Path) -> (Option<PathBuf>, Option<KeyStore>) {
    (
        Some(dir.join("config.toml")),
        Some(KeyStore::File(dir.join("keys.toml"))),
    )
}

/// The providers in `path`, or the built-in ones without it.
pub fn read(path: Option<&Path>) -> Result<Config, String> {
    Config::load(path).map_err(|error| format!("{error:#}"))
}

/// Change the file at `path` and return what it now says.
pub fn edit(
    path: &Path,
    change: impl FnOnce(&mut ConfigFile) -> anyhow::Result<()>,
) -> Result<Config, String> {
    ConfigFile::open(path)
        .and_then(|mut file| {
            change(&mut file)?;
            file.save()
        })
        .map_err(|error| format!("{error:#}"))
}

/// Save `key` for `provider`; an empty one removes it.
pub fn save_key(store: &KeyStore, provider: &str, key: &str) -> Result<(), String> {
    let result = if key.trim().is_empty() {
        store.delete(provider)
    } else {
        store.set(provider, key).map(drop)
    };
    result.map_err(|error| format!("{error:#}"))
}

/// The models `provider` lists, asked with `key`. Blocks.
pub fn list_models(provider: &ProviderConfig, key: Option<&str>) -> anyhow::Result<Vec<String>> {
    provider.list_models(key)
}
