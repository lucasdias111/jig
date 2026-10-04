//! `~/.config/jig/config.toml`: the providers Jig can reach, and the model
//! each lane uses. API keys are never in this file: they're saved from
//! Settings in the keychain (`keys.rs`) or read from environment variables.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::keys::{KeySource, KeyStore};
use crate::{AnthropicProvider, OpenAiCompatProvider, Provider};

const DEFAULT_CONFIG: &str = r#"
quick = "opencode-go/qwen3.8-flash"

[[provider]]
name = "opencode-go"
kind = "anthropic"
base_url = "https://opencode.ai/zen/go/v1"
api_key_env = "OPENCODE_API_KEY"
session_header = "x-opencode-session"
"#;

const CONFIG_HEADER: &str = "\
# Jig's AI providers, and the model each lane uses as provider/model:
# `quick` for quick commands, `agent` for agent commands (the same as quick
# when left out). Settings > Model edits this file too. API keys are never
# stored here: paste them in Settings (they go in the system keychain), or
# name an environment variable in api_key_env.
#
# kind: \"anthropic\" or \"openai\" (any OpenAI-compatible server).
# thinking = true: let the model reason before answering. Off by default;
# it makes reasoning models take 10-60 s instead of 2-5 s.

";

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    /// The model quick commands use, as `provider/model`.
    #[serde(default)]
    pub quick: Option<String>,
    /// The model agent commands use, as `provider/model`. `None` is the
    /// quick one.
    #[serde(default)]
    pub agent: Option<String>,
    /// Older files: the provider quick commands use, with its `model`.
    #[serde(default)]
    pub default: Option<String>,
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
    /// The model to call. Filled in from `quick` or `agent`; older files
    /// set it here for the `default` provider.
    #[serde(default)]
    pub model: String,
    /// Environment variable holding the API key. Omit for local servers.
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub auth: AuthStyle,
    /// Header that carries Jig's session ID. OpenCode Go requires
    /// `x-opencode-session` to route requests.
    pub session_header: Option<String>,
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// OpenAI-compatible only: request `response_format: json_object`.
    #[serde(default = "default_true")]
    pub json_mode: bool,
    /// Let the model reason before answering. Off unless set: it makes
    /// reasoning models take 10-60 s instead of 2-5 s. Turned off with
    /// `thinking: {type: disabled}` (Anthropic format) or
    /// `reasoning_effort: none` (OpenAI format).
    #[serde(default)]
    pub thinking: bool,
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_true() -> bool {
    true
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

    /// The quick lane's `provider/model`: `quick`, or in older files the
    /// `default` provider and its `model`.
    pub fn quick_model(&self) -> Option<String> {
        self.quick.clone().or_else(|| {
            let provider = self.provider(self.default.as_deref()?)?;
            (!provider.model.is_empty()).then(|| format!("{}/{}", provider.name, provider.model))
        })
    }

    /// The provider quick commands call, with its model filled in.
    pub fn quick(&self) -> Result<ProviderConfig> {
        let id = self
            .quick_model()
            .context("Pick a model for quick commands in Settings > Model.")?;
        self.resolve(&id)
    }

    /// What agent commands run on: `agent`, else an older file's
    /// `agent_model`, else the quick model.
    pub fn agent(&self) -> Result<AgentModel> {
        if self.agent.is_none()
            && let Some(id) = &self.agent_model
        {
            return Ok(AgentModel::OpenCode(id.clone()));
        }
        let id = self
            .agent
            .clone()
            .or_else(|| self.quick_model())
            .context("Pick a model for agent commands in Settings > Model.")?;
        let provider = self.resolve(&id)?;
        let model = provider.model.clone();
        Ok(AgentModel::Jig { provider, model })
    }

    fn resolve(&self, id: &str) -> Result<ProviderConfig> {
        let (name, model) = split_model(id)
            .with_context(|| format!("\"{id}\" should look like provider/model."))?;
        let provider = self.provider(name).with_context(|| {
            format!("\"{name}\" isn't connected. Pick another model in Settings > Model.")
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

    /// Build the provider with the key from the system's store or the
    /// environment.
    pub fn build(&self) -> Result<Arc<dyn Provider>> {
        self.build_with(self.system_api_key().map(|(key, _)| key))
    }

    pub fn build_with(&self, api_key: Option<String>) -> Result<Arc<dyn Provider>> {
        if self.base_url.trim().is_empty() {
            bail!("\"{}\" has no base URL. Add one in Settings.", self.name);
        }
        if self.model.trim().is_empty() {
            bail!("\"{}\" has no model. Pick one in Settings.", self.name);
        }
        if api_key.is_none() && self.needs_key() {
            let hint = match &self.api_key_env {
                Some(var) => format!("Add one in Settings, or set {var}."),
                None => "Add one in Settings.".to_string(),
            };
            bail!("\"{}\" has no API key. {hint}", self.name);
        }
        let headers: Vec<(String, String)> = self
            .session_header
            .iter()
            .map(|name| (name.clone(), session_id().to_string()))
            .collect();
        Ok(match self.kind {
            ProviderKind::Openai => Arc::new(
                OpenAiCompatProvider::new(
                    &self.base_url,
                    api_key,
                    &self.model,
                    self.max_tokens,
                    self.json_mode,
                )
                .with_headers(headers)
                .with_thinking(self.thinking),
            ),
            ProviderKind::Anthropic => Arc::new(
                AnthropicProvider::new(
                    &self.base_url,
                    api_key.unwrap_or_default(),
                    &self.model,
                    self.max_tokens,
                    self.auth,
                )
                .with_headers(headers)
                .with_thinking(self.thinking),
            ),
        })
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

    #[test]
    fn built_in_uses_opencode_go() {
        let quick = Config::built_in().quick().unwrap();
        assert_eq!(quick.name, "opencode-go");
        assert_eq!(quick.model, "qwen3.8-flash");
        assert_eq!(quick.kind, ProviderKind::Anthropic);
        assert_eq!(quick.auth, AuthStyle::ApiKey);
        assert_eq!(quick.api_key_env.as_deref(), Some("OPENCODE_API_KEY"));
        assert_eq!(quick.session_header.as_deref(), Some("x-opencode-session"));
        assert_eq!(quick.max_tokens, 4096);
        assert_eq!(session_id(), session_id());
    }

    #[test]
    fn each_lane_has_its_own_model() {
        let config = parse(&format!(
            "quick = \"anthropic/claude-haiku-4-5\"\nagent = \"openrouter/anthropic/claude-sonnet-5-5\"\n{TWO}"
        ));
        assert_eq!(config.quick().unwrap().model, "claude-haiku-4-5");
        let AgentModel::Jig { provider, model } = config.agent().unwrap() else {
            panic!("a Jig provider");
        };
        assert_eq!(provider.name, "openrouter");
        assert_eq!(model, "anthropic/claude-sonnet-5-5");
    }

    #[test]
    fn the_agent_follows_quick_unless_set() {
        let config = parse(&format!("quick = \"anthropic/claude-haiku-4-5\"\n{TWO}"));
        let AgentModel::Jig { provider, model } = config.agent().unwrap() else {
            panic!("a Jig provider");
        };
        assert_eq!(
            (provider.name.as_str(), model.as_str()),
            ("anthropic", "claude-haiku-4-5")
        );
    }

    #[test]
    fn older_files_still_work() {
        let config = parse(
            "default = \"claude\"\nagent_model = \"opencode-go/glm-5.3-flash\"\n\
             [[provider]]\nname = \"claude\"\nkind = \"anthropic\"\n\
             base_url = \"u\"\nmodel = \"claude-sonnet-5-5\"\n",
        );
        assert_eq!(
            config.quick_model().as_deref(),
            Some("claude/claude-sonnet-5-5")
        );
        assert!(matches!(
            config.agent().unwrap(),
            AgentModel::OpenCode(id) if id == "opencode-go/glm-5.3-flash"
        ));
    }

    #[test]
    fn unpicked_or_disconnected_models_are_clear_errors() {
        let none = parse(TWO);
        assert_eq!(
            none.quick().unwrap_err().to_string(),
            "Pick a model for quick commands in Settings > Model."
        );
        let gone = parse(&format!("quick = \"groq/llama\"\n{TWO}"));
        assert_eq!(
            gone.quick().unwrap_err().to_string(),
            "\"groq\" isn't connected. Pick another model in Settings > Model."
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
    fn missing_key_is_a_clear_error() {
        let error = Config::built_in()
            .quick()
            .unwrap()
            .build_with(None)
            .err()
            .unwrap();
        assert_eq!(
            error.to_string(),
            "\"opencode-go\" has no API key. Add one in Settings, or set OPENCODE_API_KEY."
        );
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
        let config = parse(&format!(
            "quick = \"openrouter/m\"\n{}",
            TWO.replace("openrouter.ai/api", "localhost:11434")
        ));
        assert!(config.quick().unwrap().build_with(None).is_ok());
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
