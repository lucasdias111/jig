//! `~/.config/jig/config.toml`: which providers exist and which one to use.
//! API keys are read from environment variables, never from this file.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Deserialize;

use crate::{AnthropicProvider, OpenAiCompatProvider, Provider};

const DEFAULT_CONFIG: &str = r#"
default = "opencode-qwen"

[[provider]]
name = "opencode-glm"
kind = "openai"
base_url = "https://opencode.ai/zen/go/v1"
model = "glm-5.3-flash"
api_key_env = "OPENCODE_API_KEY"
session_header = "x-opencode-session"

[[provider]]
name = "opencode-qwen"
kind = "anthropic"
base_url = "https://opencode.ai/zen/go/v1"
model = "qwen3.8-flash"
api_key_env = "OPENCODE_API_KEY"
session_header = "x-opencode-session"

[[provider]]
name = "claude"
kind = "anthropic"
base_url = "https://api.anthropic.com/v1"
model = "claude-sonnet-5-5"
api_key_env = "ANTHROPIC_API_KEY"

[[provider]]
name = "ollama"
kind = "openai"
base_url = "http://localhost:11434/v1"
model = "qwen2.5-coder:7b"
"#;

const CONFIG_HEADER: &str = "\
# Jig's AI providers. `default` is the one commands use unless you pick
# another in Settings. API keys come from the environment variable named in
# api_key_env, never from this file.
#
# kind: \"anthropic\" or \"openai\" (any OpenAI-compatible server).

";

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    /// Name of the provider commands use.
    pub default: String,
    #[serde(rename = "provider")]
    pub providers: Vec<ProviderConfig>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Anthropic,
    Openai,
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
}

fn default_max_tokens() -> u32 {
    4096
}

fn default_true() -> bool {
    true
}

impl Config {
    pub fn parse(source: &str) -> Result<Self> {
        let config: Config = toml::from_str(source)?;
        if !config.providers.iter().any(|p| p.name == config.default) {
            bail!("default provider \"{}\" is not defined", config.default);
        }
        Ok(config)
    }

    pub fn built_in() -> Self {
        Self::parse(DEFAULT_CONFIG).expect("built-in config is valid")
    }

    /// `~/.config/jig/config.toml`, honouring `XDG_CONFIG_HOME`.
    pub fn user_path() -> Option<PathBuf> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
        Some(config.join("jig").join("config.toml"))
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

    pub fn default_provider(&self) -> &ProviderConfig {
        self.provider(&self.default).expect("checked in parse")
    }

    pub fn provider(&self, name: &str) -> Option<&ProviderConfig> {
        self.providers.iter().find(|p| p.name == name)
    }

    /// The provider named `choice` (the user's pick in Settings), or the
    /// file's default when there is no pick or it no longer exists.
    pub fn chosen_provider(&self, choice: Option<&str>) -> &ProviderConfig {
        choice
            .and_then(|name| self.provider(name))
            .unwrap_or_else(|| self.default_provider())
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

impl ProviderConfig {
    /// Whether the API key this provider needs is set. Local servers need
    /// none.
    pub fn has_key(&self) -> bool {
        self.api_key_env
            .as_deref()
            .is_none_or(|var| std::env::var(var).is_ok_and(|key| !key.trim().is_empty()))
    }
}

impl ProviderConfig {
    /// Build the provider, reading its API key from the environment.
    pub fn build(&self) -> Result<Arc<dyn Provider>> {
        self.build_with(|name| std::env::var(name).ok())
    }

    pub fn build_with(&self, env: impl Fn(&str) -> Option<String>) -> Result<Arc<dyn Provider>> {
        let api_key =
            match &self.api_key_env {
                Some(var) => Some(env(var).filter(|key| !key.trim().is_empty()).ok_or_else(
                    || anyhow!("{var} is not set (needed by provider \"{}\").", self.name),
                )?),
                None => None,
            };
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
                .with_headers(headers),
            ),
            ProviderKind::Anthropic => Arc::new(
                AnthropicProvider::new(
                    &self.base_url,
                    api_key.context("Anthropic-format providers need api_key_env.")?,
                    &self.model,
                    self.max_tokens,
                    self.auth,
                )
                .with_headers(headers),
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

    #[test]
    fn built_in_defaults_to_opencode() {
        let config = Config::built_in();
        let provider = config.default_provider();
        assert_eq!(provider.name, "opencode-qwen");
        assert_eq!(provider.model, "qwen3.8-flash");
        assert_eq!(provider.kind, ProviderKind::Anthropic);
        assert_eq!(provider.auth, AuthStyle::ApiKey);
        assert_eq!(provider.api_key_env.as_deref(), Some("OPENCODE_API_KEY"));
        assert_eq!(provider.max_tokens, 4096);
    }

    #[test]
    fn unknown_default_is_an_error() {
        let source = "default = \"x\"\n[[provider]]\nname = \"y\"\nkind = \"openai\"\nbase_url = \"u\"\nmodel = \"m\"\n";
        assert!(Config::parse(source).is_err());
    }

    #[test]
    fn missing_key_is_a_clear_error() {
        let config = Config::built_in();
        let error = config
            .default_provider()
            .build_with(|_| None)
            .err()
            .unwrap();
        assert!(error.to_string().contains("OPENCODE_API_KEY is not set"));
    }

    #[test]
    fn opencode_providers_send_a_session_header() {
        let config = Config::built_in();
        for provider in config
            .providers
            .iter()
            .filter(|p| p.base_url.contains("opencode.ai"))
        {
            assert_eq!(
                provider.session_header.as_deref(),
                Some("x-opencode-session"),
                "{}",
                provider.name
            );
        }
        assert_eq!(session_id(), session_id());
    }

    #[test]
    fn chosen_provider_falls_back_to_the_default() {
        let config = Config::built_in();
        assert_eq!(config.chosen_provider(Some("claude")).name, "claude");
        assert_eq!(config.chosen_provider(Some("gone")).name, "opencode-qwen");
        assert_eq!(config.chosen_provider(None).name, "opencode-qwen");
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
        let config = Config::built_in();
        let ollama = config
            .providers
            .iter()
            .find(|p| p.name == "ollama")
            .unwrap();
        assert!(ollama.build_with(|_| None).is_ok());
    }
}
