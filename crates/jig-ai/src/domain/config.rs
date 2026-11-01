//! `~/.config/jig/config.toml`: the providers Jig can reach, and the model
//! the agent uses. API keys are never in this file: they're saved from
//! Settings in the keychain (`keys.rs`) or read from environment variables.

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

/// No provider to begin with: Jig favours none. Connecting the first one
/// in Settings picks a model for the agent.
/// `agent` is there, empty, so the header stays above what Settings
/// writes later.
pub(crate) const DEFAULT_CONFIG: &str = "\
# No provider yet: connect one in Settings > Providers.
agent = \"\"
";

/// What to say while nothing is connected.
const CONNECT_FIRST: &str = "Connect a provider in Settings > Providers first.";

pub(crate) const CONFIG_HEADER: &str = "\
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
}

impl ProviderConfig {
    /// Whether the provider can't work without a key. Local
    /// OpenAI-compatible servers need none.
    pub fn needs_key(&self) -> bool {
        self.kind == ProviderKind::Anthropic || self.api_key_env.is_some()
    }
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
        let id = crate::store::session::session_id;
        assert_eq!(id(), id());
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
    fn local_provider_needs_no_key() {
        let config = parse(&TWO.replace("openrouter.ai/api", "localhost:11434"));
        assert!(config.provider("anthropic").unwrap().needs_key());
        assert!(!config.provider("openrouter").unwrap().needs_key());
    }
}
