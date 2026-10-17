//! `~/.config/jig/config.toml`: the providers Jig can reach, and the model
//! the agent uses. API keys are never in this file: they're saved from
//! Settings in the keychain (`keys.rs`) or read from environment variables.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::keys::{KeySource, KeyStore};

/// No provider to begin with: Jig favours none. Connecting the first one
/// in Settings picks a model for the agent.
/// `agent` is there, empty, so the header stays above what Settings
/// writes later.
const DEFAULT_CONFIG: &str = "\
# No provider yet: connect one in Settings > Providers.
agent = \"\"
";

/// What to say while nothing is connected.
const CONNECT_FIRST: &str = "Connect a provider in Settings > Providers first.";

const CONFIG_HEADER: &str = "\
# Jig's AI providers, and the model the agent uses as provider/model in
# `agent`. Settings > Providers and Agent edit this file too. API keys
# are never stored here: paste them in Settings (they go in the system
# keychain), or name an environment variable in api_key_env.
#
# kind: \"anthropic\" or \"openai\" (any OpenAI-compatible server).

";

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    /// The model the agent uses, as `provider/model`.
    #[serde(default)]
    pub agent: Option<String>,
    /// Older files: the agent's model as OpenCode itself names it.
    #[serde(default)]
    pub agent_model: Option<String>,
    #[serde(rename = "provider", default)]
    pub providers: Vec<ProviderConfig>,
}

/// What agent commands run on.
#[derive(Clone, Debug)]
pub enum AgentModel {
    /// One of Jig's providers, handed to OpenCode when it starts.
    Jig {
        provider: ProviderConfig,
        model: String,
    },
    /// A model OpenCode knows by itself, from `agent_model`.
    OpenCode(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Anthropic,
    Openai,
}

impl ProviderKind {
    /// As written in `config.toml`.
    pub fn key(self) -> &'static str {
        match self {
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::Openai => "openai",
        }
    }
}

/// How an Anthropic-format provider expects the key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthStyle {
    /// `x-api-key: <key>` (Anthropic's API).
    #[default]
    ApiKey,
    /// `Authorization: Bearer <key>`.
    Bearer,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub kind: ProviderKind,
    pub base_url: String,
    /// The model to call, filled in from `agent`.
    #[serde(default)]
    pub model: String,
    /// Environment variable holding the API key. Omit for local servers.
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub auth: AuthStyle,
    /// Header that carries Jig's session ID. OpenCode Go requires
    /// `x-opencode-session` to route requests.
    pub session_header: Option<String>,
}

/// Split `provider/model` at the first slash: model IDs have their own, as
/// in OpenRouter's `anthropic/claude-haiku-4-5`.
pub fn split_model(id: &str) -> Option<(&str, &str)> {
    id.split_once('/')
        .filter(|(provider, model)| !provider.is_empty() && !model.is_empty())
}

impl Config {
    pub fn parse(source: &str) -> Result<Self> {
        let config: Config = toml::from_str(source)?;
        for (index, provider) in config.providers.iter().enumerate() {
            if provider.name.is_empty() || provider.name.contains('/') {
                bail!(
                    "provider name \"{}\" must be non-empty and have no slash",
                    provider.name
                );
            }
            if config.providers[..index]
                .iter()
                .any(|other| other.name == provider.name)
            {
                bail!("there are two providers called \"{}\"", provider.name);
            }
        }
        Ok(config)
    }

    pub fn built_in() -> Self {
        Self::parse(DEFAULT_CONFIG).expect("built-in config is valid")
    }

    /// `~/.config/jig/config.toml`, honouring `XDG_CONFIG_HOME`.
    pub fn user_path() -> Option<PathBuf> {
        Some(config_dir()?.join("config.toml"))
    }

    /// The user's config if it exists, otherwise the built-in one.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        match path.filter(|path| path.exists()) {
            Some(path) => {
                let source = std::fs::read_to_string(path)
                    .with_context(|| format!("reading {}", path.display()))?;
                Self::parse(&source).with_context(|| format!("parsing {}", path.display()))
            }
            None => Ok(Self::built_in()),
        }
    }

    pub fn provider(&self, name: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.name == name)
    }

    /// What the agent runs on: `agent`, else an older file's
    /// `agent_model`.
    pub fn agent(&self) -> Result<AgentModel> {
        if self.agent.is_none()
            && let Some(id) = &self.agent_model
        {
            return Ok(AgentModel::OpenCode(id.clone()));
        }
        if self.providers.is_empty() {
            bail!("{CONNECT_FIRST}");
        }
        let id = self
            .agent
            .clone()
            // An empty one is the built-in file's: none picked yet.
            .filter(|id| !id.is_empty())
            .context("Pick a model for the agent in Settings > Agent.")?;
        let provider = self.resolve(&id)?;
        let model = provider.model.clone();
        Ok(AgentModel::Jig { provider, model })
    }

    /// The provider for `provider/model`, with its model filled in.
    pub fn resolve(&self, id: &str) -> Result<ProviderConfig> {
        let (name, model) = split_model(id)
            .with_context(|| format!("\"{id}\" should look like provider/model."))?;
        let provider = self.provider(name).with_context(|| {
            format!("\"{name}\" isn't connected. Connect it in Settings > Providers, or pick another model.")
        })?;
        Ok(ProviderConfig {
            model: model.to_string(),
            ..provider.clone()
        })
    }

    /// Write the built-in config to `path` if there is nothing there yet,
    /// so it can be edited.
    pub fn ensure_user_file(path: &Path) -> Result<()> {
        if path.exists() {
            return Ok(());
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let source = format!("{CONFIG_HEADER}{}", DEFAULT_CONFIG.trim_start());
        std::fs::write(path, source).with_context(|| format!("writing {}", path.display()))
    }
}

/// Jig's config folder, `~/.config/jig`, honouring `XDG_CONFIG_HOME`.
pub(crate) fn config_dir() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("jig"))
}

fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

impl ProviderConfig {
    /// The key to send and where it came from: the one saved in Settings,
    /// otherwise the `api_key_env` variable.
    pub fn api_key(
        &self,
        store: Option<&KeyStore>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Option<(String, KeySource)> {
        store.and_then(|store| store.get(&self.name)).or_else(|| {
            let var = self.api_key_env.as_deref()?;
            let key = env(var).filter(|key| !key.trim().is_empty())?;
            Some((key, KeySource::Env(var.to_string())))
        })
    }

    /// The key from the system's store or the environment.
    pub fn system_api_key(&self) -> Option<(String, KeySource)> {
        self.api_key(KeyStore::system().as_ref(), env_var)
    }

    /// Whether the provider can't work without a key. Local
    /// OpenAI-compatible servers need none.
    pub fn needs_key(&self) -> bool {
        self.kind == ProviderKind::Anthropic || self.api_key_env.is_some()
    }
}

/// One ID per Jig launch, stable across that run's requests.
pub fn session_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_nanos());
        format!("jig-{:x}-{nanos:x}", std::process::id())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(source: &str) -> Config {
        Config::parse(source).unwrap()
    }

    const TWO: &str = "\
[[provider]]
name = \"anthropic\"
kind = \"anthropic\"
base_url = \"https://api.anthropic.com/v1\"
api_key_env = \"ANTHROPIC_API_KEY\"

[[provider]]
name = \"openrouter\"
kind = \"openai\"
base_url = \"https://openrouter.ai/api/v1\"
";

    /// OpenCode Go, as Settings connects it.
    const OPENCODE_GO: &str = "\
agent = \"opencode-go/qwen3.8-flash\"

[[provider]]
name = \"opencode-go\"
kind = \"anthropic\"
base_url = \"https://opencode.ai/zen/go/v1\"
api_key_env = \"OPENCODE_API_KEY\"
session_header = \"x-opencode-session\"
";

    fn jig_provider(config: &Config) -> (ProviderConfig, String) {
        match config.agent().unwrap() {
            AgentModel::Jig { provider, model } => (provider, model),
            AgentModel::OpenCode(id) => panic!("a Jig provider, not {id}"),
        }
    }

    #[test]
    fn built_in_favours_no_provider() {
        let config = Config::built_in();
        assert!(config.providers.is_empty());
        assert_eq!(config.agent().unwrap_err().to_string(), CONNECT_FIRST);
    }

    #[test]
    fn a_provider_fills_in_its_defaults() {
        let (provider, model) = jig_provider(&parse(OPENCODE_GO));
        assert_eq!(provider.name, "opencode-go");
        assert_eq!(model, "qwen3.8-flash");
        assert_eq!(provider.model, "qwen3.8-flash");
        assert_eq!(provider.kind, ProviderKind::Anthropic);
        assert_eq!(provider.auth, AuthStyle::ApiKey);
        assert_eq!(provider.api_key_env.as_deref(), Some("OPENCODE_API_KEY"));
        assert_eq!(
            provider.session_header.as_deref(),
            Some("x-opencode-session")
        );
        assert_eq!(session_id(), session_id());
    }

    #[test]
    fn a_model_id_keeps_its_own_slashes() {
        let config = parse(&format!(
            "agent = \"openrouter/anthropic/claude-sonnet-5-5\"\n{TWO}"
        ));
        let (provider, model) = jig_provider(&config);
        assert_eq!(provider.name, "openrouter");
        assert_eq!(model, "anthropic/claude-sonnet-5-5");
    }

    #[test]
    fn older_files_still_work() {
        let config = parse(
            "agent_model = \"opencode-go/glm-5.3-flash\"\n\
             [[provider]]\nname = \"claude\"\nkind = \"anthropic\"\n\
             base_url = \"u\"\nmodel = \"claude-sonnet-5-5\"\nmax_tokens = 4096\n",
        );
        assert!(matches!(
            config.agent().unwrap(),
            AgentModel::OpenCode(id) if id == "opencode-go/glm-5.3-flash"
        ));
    }

    #[test]
    fn the_retired_jig_mode_model_is_ignored() {
        let config = parse(&format!("quick = \"anthropic/claude-haiku-4-5\"\n{TWO}"));
        assert_eq!(
            config.agent().unwrap_err().to_string(),
            "Pick a model for the agent in Settings > Agent."
        );
    }

    #[test]
    fn unpicked_or_disconnected_models_are_clear_errors() {
        let none = parse(TWO);
        assert_eq!(
            none.agent().unwrap_err().to_string(),
            "Pick a model for the agent in Settings > Agent."
        );
        let gone = parse(&format!("agent = \"groq/llama\"\n{TWO}"));
        assert_eq!(
            gone.agent().unwrap_err().to_string(),
            "\"groq\" isn't connected. Connect it in Settings > Providers, or pick another model."
        );
    }

    #[test]
    fn provider_names_are_unique_and_slash_free() {
        let twice = format!(
            "{TWO}{}",
            &TWO[TWO.find("[[provider]]\nname = \"openrouter").unwrap()..]
        );
        assert!(Config::parse(&twice).is_err());
        let slash = TWO.replace("\"openrouter\"", "\"open/router\"");
        assert!(Config::parse(&slash).is_err());
        assert_eq!(split_model("openrouter/a/b"), Some(("openrouter", "a/b")));
        assert_eq!(split_model("openrouter/"), None);
    }

    #[test]
    fn user_file_starts_as_the_built_in_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jig").join("config.toml");
        Config::ensure_user_file(&path).unwrap();
        let config = Config::load(Some(&path)).unwrap();
        assert_eq!(config.providers.len(), Config::built_in().providers.len());
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with("# Jig's AI")
        );

        // An existing file is left alone.
        std::fs::write(&path, "mine").unwrap();
        Config::ensure_user_file(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine");
    }

    #[test]
    fn local_provider_needs_no_key() {
        let config = parse(&TWO.replace("openrouter.ai/api", "localhost:11434"));
        assert!(config.provider("anthropic").unwrap().needs_key());
        assert!(!config.provider("openrouter").unwrap().needs_key());
    }

    #[test]
    fn a_saved_key_wins_over_the_environment() {
        let dir = tempfile::tempdir().unwrap();
        let store = KeyStore::File(dir.path().join("keys.toml"));
        let config = parse(TWO);
        let anthropic = config.provider("anthropic").unwrap();
        let env = |_: &str| Some("from-env".to_string());

        let (key, source) = anthropic.api_key(Some(&store), env).unwrap();
        assert_eq!(key, "from-env");
        assert_eq!(source, KeySource::Env("ANTHROPIC_API_KEY".into()));

        store.set("anthropic", "saved").unwrap();
        assert_eq!(
            anthropic.api_key(Some(&store), env),
            Some(("saved".into(), KeySource::File))
        );
    }
}
