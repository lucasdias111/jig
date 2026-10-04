//! API keys typed into Settings. They go in the system keychain where there
//! is one (macOS Keychain, Windows Credential Manager), otherwise in
//! `keys.toml` next to `config.toml`, readable only by the user. Never in
//! `config.toml` itself, which people share with their dotfiles.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use anyhow::{Context as _, Result};

/// Where a saved key lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeyStore {
    /// The system keychain, with the file as the fallback when it fails.
    #[cfg_attr(not(any(target_os = "macos", target_os = "windows")), allow(dead_code))]
    Keychain(PathBuf),
    File(PathBuf),
}

/// Where the key a provider uses comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KeySource {
    Keychain,
    File,
    /// The environment variable named in `api_key_env`.
    Env(String),
}

impl KeySource {
    /// How Settings describes it.
    pub fn label(&self) -> String {
        match self {
            KeySource::Keychain if cfg!(target_os = "macos") => "Saved in Keychain".into(),
            KeySource::Keychain => "Saved in Credential Manager".into(),
            KeySource::File => "Saved in keys.toml".into(),
            KeySource::Env(var) => format!("From {var}"),
        }
    }
}

/// Keys already read this run, by store file and provider, so building a
/// provider doesn't ask the keychain again, which can prompt on macOS.
type Cache = HashMap<(PathBuf, String), Option<(String, KeySource)>>;

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

impl KeyStore {
    /// The keychain where the system has one, otherwise `keys.toml` in
    /// Jig's config folder.
    pub fn system() -> Option<Self> {
        let file = crate::config::config_dir()?.join("keys.toml");
        if cfg!(any(target_os = "macos", target_os = "windows")) {
            Some(KeyStore::Keychain(file))
        } else {
            Some(KeyStore::File(file))
        }
    }

    fn file(&self) -> &Path {
        match self {
            KeyStore::Keychain(file) | KeyStore::File(file) => file,
        }
    }

    /// The saved key for `provider`, if there is one.
    pub fn get(&self, provider: &str) -> Option<(String, KeySource)> {
        let id = (self.file().to_path_buf(), provider.to_string());
        let mut cache = cache().lock().unwrap_or_else(|error| error.into_inner());
        cache
            .entry(id)
            .or_insert_with(|| self.read(provider))
            .clone()
    }

    fn read(&self, provider: &str) -> Option<(String, KeySource)> {
        if let KeyStore::Keychain(_) = self
            && let Some(key) = keychain::get(provider)
        {
            return Some((key, KeySource::Keychain));
        }
        let key = read_file(self.file()).ok()?.remove(provider)?;
        Some((key, KeySource::File))
    }

    /// Save `key` for `provider`, replacing any saved before.
    pub fn set(&self, provider: &str, key: &str) -> Result<KeySource> {
        let key = key.trim();
        let source = match self {
            KeyStore::Keychain(_) if keychain::set(provider, key).is_ok() => {
                // An older copy in the file would be found if the keychain
                // ever failed to answer.
                self.remove_from_file(provider)?;
                KeySource::Keychain
            }
            _ => {
                let mut keys = read_file(self.file())?;
                keys.insert(provider.to_string(), key.to_string());
                write_file(self.file(), &keys)?;
                KeySource::File
            }
        };
        self.remember(provider, Some((key.to_string(), source.clone())));
        Ok(source)
    }

    /// Forget the saved key for `provider`, wherever it is.
    pub fn delete(&self, provider: &str) -> Result<()> {
        if let KeyStore::Keychain(_) = self {
            keychain::delete(provider)?;
        }
        self.remove_from_file(provider)?;
        self.remember(provider, None);
        Ok(())
    }

    fn remove_from_file(&self, provider: &str) -> Result<()> {
        let mut keys = read_file(self.file())?;
        if keys.remove(provider).is_some() {
            write_file(self.file(), &keys)?;
        }
        Ok(())
    }

    fn remember(&self, provider: &str, key: Option<(String, KeySource)>) {
        let id = (self.file().to_path_buf(), provider.to_string());
        let mut cache = cache().lock().unwrap_or_else(|error| error.into_inner());
        cache.insert(id, key);
    }
}

fn read_file(path: &Path) -> Result<BTreeMap<String, String>> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let source =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&source).with_context(|| format!("parsing {}", path.display()))
}

/// Write the keys readable only by the user.
fn write_file(path: &Path, keys: &BTreeMap<String, String>) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    let source = format!(
        "# API keys saved from Jig's Settings. Keep this file private.\n\n{}",
        toml::to_string(keys)?
    );
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};
        options.mode(0o600);
        // `mode` only applies to a new file.
        if path.exists() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    use std::io::Write as _;
    options
        .open(path)
        .and_then(|mut file| file.write_all(source.as_bytes()))
        .with_context(|| format!("writing {}", path.display()))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod keychain {
    use anyhow::Result;

    const SERVICE: &str = "Jig";

    fn entry(provider: &str) -> keyring::Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, provider)
    }

    pub fn get(provider: &str) -> Option<String> {
        entry(provider).and_then(|entry| entry.get_password()).ok()
    }

    pub fn set(provider: &str, key: &str) -> Result<()> {
        Ok(entry(provider)?.set_password(key)?)
    }

    pub fn delete(provider: &str) -> Result<()> {
        match entry(provider)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod keychain {
    use anyhow::{Result, bail};

    pub fn get(_: &str) -> Option<String> {
        None
    }

    pub fn set(_: &str, _: &str) -> Result<()> {
        bail!("no keychain on this system")
    }

    pub fn delete(_: &str) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_store_round_trips_and_stays_private() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::File(dir.path().join("keys.toml"));
        assert_eq!(store.get("claude"), None);

        assert_eq!(store.set("claude", " sk-1 \n").unwrap(), KeySource::File);
        assert_eq!(store.get("claude"), Some(("sk-1".into(), KeySource::File)));
        store.set("openai", "sk-2").unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(store.file())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        store.delete("claude").unwrap();
        assert_eq!(store.get("claude"), None);
        assert_eq!(read_file(store.file()).unwrap().len(), 1);
    }
}
