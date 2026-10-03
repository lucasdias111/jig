//! `~/.config/jig/config.toml`: which providers exist and which one to use.
//! API keys are read from environment variables, never from this file.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use serde::Deserialize;

use crate::{AnthropicProvider, OpenAiCompatProvider, Provider};

const DEFAULT_CONFIG: &str = r#"
default = "opencode-glm"

[[provider]]
name = "opencode-glm"
kind = "openai"
base_url = "https://opencode.ai/zen/go/v1"
model = "glm-5.3-flash"
api_key_env = "OPENCODE_API_KEY"

[[provider]]
name = "opencode-qwen"
kind = "anthropic"
base_url = "https://opencode.ai/zen/go/v1"
model = "qwen3.8-flash"
api_key_env = "OPENCODE_API_KEY"
auth = "bearer"

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
        self.providers
            .iter()
            .find(|p| p.name == self.default)
            .expect("checked in parse")
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
        Ok(match self.kind {
            ProviderKind::Openai => Arc::new(OpenAiCompatProvider::new(
                &self.base_url,
                api_key,
                &self.model,
                self.max_tokens,
                self.json_mode,
            )),
            ProviderKind::Anthropic => Arc::new(AnthropicProvider::new(
                &self.base_url,
                api_key.context("Anthropic-format providers need api_key_env.")?,
                &self.model,
                self.max_tokens,
                self.auth,
            )),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_defaults_to_opencode() {
        let config = Config::built_in();
        let provider = config.default_provider();
        assert_eq!(provider.kind, ProviderKind::Openai);
        assert_eq!(provider.api_key_env.as_deref(), Some("OPENCODE_API_KEY"));
        assert!(provider.json_mode);
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
