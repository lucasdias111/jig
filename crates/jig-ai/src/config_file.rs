//! Changing `config.toml` from Settings. Edits go through `toml_edit`, so
//! the user's comments and layout survive, and a file that doesn't parse is
//! never written over.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use toml_edit::{ArrayOfTables, DocumentMut, Table, value};

use crate::config::{Config, ProviderKind, split_model};

/// A major provider Settings offers to connect.
#[derive(Clone, Copy, Debug)]
pub struct ProviderTemplate {
    pub label: &'static str,
    /// The provider's name in `config.toml`, and in `provider/model`.
    pub name: &'static str,
    pub kind: ProviderKind,
    pub base_url: &'static str,
    /// `None` for a server on this computer, which needs no key.
    pub api_key_env: Option<&'static str>,
    pub session_header: Option<&'static str>,
}

const fn template(
    label: &'static str,
    name: &'static str,
    kind: ProviderKind,
    base_url: &'static str,
    api_key_env: Option<&'static str>,
) -> ProviderTemplate {
    ProviderTemplate {
        label,
        name,
        kind,
        base_url,
        api_key_env,
        session_header: None,
    }
}

/// The providers Settings lists. Almost every provider speaks one of the
/// two formats, so these differ only in where they point.
pub const TEMPLATES: &[ProviderTemplate] = &[
    template(
        "Anthropic",
        "anthropic",
        ProviderKind::Anthropic,
        "https://api.anthropic.com/v1",
        Some("ANTHROPIC_API_KEY"),
    ),
    template(
        "OpenAI",
        "openai",
        ProviderKind::Openai,
        "https://api.openai.com/v1",
        Some("OPENAI_API_KEY"),
    ),
    template(
        "Google Gemini",
        "gemini",
        ProviderKind::Openai,
        "https://generativelanguage.googleapis.com/v1beta/openai",
        Some("GEMINI_API_KEY"),
    ),
    template(
        "OpenRouter",
        "openrouter",
        ProviderKind::Openai,
        "https://openrouter.ai/api/v1",
        Some("OPENROUTER_API_KEY"),
    ),
    template(
        "Groq",
        "groq",
        ProviderKind::Openai,
        "https://api.groq.com/openai/v1",
        Some("GROQ_API_KEY"),
    ),
    template(
        "Mistral",
        "mistral",
        ProviderKind::Openai,
        "https://api.mistral.ai/v1",
        Some("MISTRAL_API_KEY"),
    ),
    template(
        "DeepSeek",
        "deepseek",
        ProviderKind::Openai,
        "https://api.deepseek.com/v1",
        Some("DEEPSEEK_API_KEY"),
    ),
    ProviderTemplate {
        session_header: Some("x-opencode-session"),
        ..template(
            "OpenCode Go",
            "opencode-go",
            ProviderKind::Anthropic,
            "https://opencode.ai/zen/go/v1",
            Some("OPENCODE_API_KEY"),
        )
    },
    template(
        "Ollama",
        "ollama",
        ProviderKind::Openai,
        "http://localhost:11434/v1",
        None,
    ),
    template(
        "LM Studio",
        "lm-studio",
        ProviderKind::Openai,
        "http://localhost:1234/v1",
        None,
    ),
];

/// The template a provider was made from, by name.
pub fn template_named(name: &str) -> Option<&'static ProviderTemplate> {
    TEMPLATES.iter().find(|template| template.name == name)
}

/// The user's `config.toml`, open for editing.
pub struct ConfigFile {
    path: PathBuf,
    doc: DocumentMut,
}

impl ConfigFile {
    /// Open the file, writing the built-in providers there first if it
    /// doesn't exist yet.
    pub fn open(path: &Path) -> Result<Self> {
        Config::ensure_user_file(path)?;
        let source =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Config::parse(&source).with_context(|| format!("parsing {}", path.display()))?;
        let doc = source
            .parse::<DocumentMut>()
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self {
            path: path.to_path_buf(),
            doc,
        })
    }

    /// Write the file and return what it now says. Refuses to write
    /// anything Jig couldn't read back.
    pub fn save(&self) -> Result<Config> {
        let source = self.doc.to_string();
        let config = Config::parse(&source)?;
        std::fs::write(&self.path, source)
            .with_context(|| format!("writing {}", self.path.display()))?;
        Ok(config)
    }

    fn providers(&mut self) -> Result<&mut ArrayOfTables> {
        self.doc
            .get_mut("provider")
            .and_then(|item| item.as_array_of_tables_mut())
            .context("config.toml has no [[provider]] tables")
    }

    fn provider(&mut self, name: &str) -> Result<&mut Table> {
        self.providers()?
            .iter_mut()
            .find(|table| table.get("name").and_then(|item| item.as_str()) == Some(name))
            .with_context(|| format!("There's no provider called \"{name}\"."))
    }

    fn names(&mut self) -> Result<Vec<String>> {
        Ok(self
            .providers()?
            .iter()
            .filter_map(|table| table.get("name")?.as_str().map(str::to_string))
            .collect())
    }

    /// Add `template`'s provider unless it's there already.
    pub fn connect(&mut self, template: &ProviderTemplate) -> Result<()> {
        if self.names()?.iter().any(|name| name == template.name) {
            return Ok(());
        }
        let mut table = Table::new();
        table["name"] = value(template.name);
        table["kind"] = value(template.kind.key());
        table["base_url"] = value(template.base_url);
        if let Some(var) = template.api_key_env {
            table["api_key_env"] = value(var);
        }
        if let Some(header) = template.session_header {
            table["session_header"] = value(header);
        }
        self.providers()?.push(table);
        Ok(())
    }

    /// Set the model quick commands use, as `provider/model`.
    pub fn set_quick(&mut self, id: &str) -> Result<()> {
        self.check_model(id)?;
        self.doc["quick"] = value(id);
        // An older file's way of saying it would only confuse.
        self.doc.remove("default");
        Ok(())
    }

    /// Set the model agent commands use; `None` follows quick commands.
    pub fn set_agent(&mut self, id: Option<&str>) -> Result<()> {
        match id {
            Some(id) => {
                self.check_model(id)?;
                self.doc["agent"] = value(id);
            }
            None => {
                self.doc.remove("agent");
            }
        }
        self.doc.remove("agent_model");
        Ok(())
    }

    fn check_model(&mut self, id: &str) -> Result<()> {
        let (name, _) = split_model(id)
            .with_context(|| format!("\"{id}\" should look like provider/model."))?;
        self.provider(name)?;
        Ok(())
    }

    /// Remove a provider. A lane that used it is left without a model, so
    /// Settings asks for another.
    pub fn remove(&mut self, name: &str) -> Result<()> {
        let providers = self.providers()?;
        let index = providers
            .iter()
            .position(|table| table.get("name").and_then(|item| item.as_str()) == Some(name))
            .with_context(|| format!("There's no provider called \"{name}\"."))?;
        providers.remove(index);
        let prefix = format!("{name}/");
        for lane in ["quick", "agent"] {
            if self
                .doc
                .get(lane)
                .and_then(|item| item.as_str())
                .is_some_and(|id| id.starts_with(&prefix))
            {
                self.doc.remove(lane);
            }
        }
        if self.doc.get("default").and_then(|item| item.as_str()) == Some(name) {
            self.doc.remove("default");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentModel;

    fn open_temp() -> (tempfile::TempDir, PathBuf, ConfigFile) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let file = ConfigFile::open(&path).unwrap();
        (dir, path, file)
    }

    fn connect(file: &mut ConfigFile, name: &str) {
        file.connect(template_named(name).unwrap()).unwrap();
    }

    #[test]
    fn edits_keep_the_users_comments() {
        let (_dir, path, mut file) = open_temp();
        connect(&mut file, "anthropic");
        file.set_quick("anthropic/claude-haiku-4-5").unwrap();
        let config = file.save().unwrap();
        assert_eq!(config.quick().unwrap().model, "claude-haiku-4-5");
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .starts_with("# Jig's AI")
        );
    }

    #[test]
    fn connecting_twice_adds_one_provider() {
        let (_dir, _path, mut file) = open_temp();
        connect(&mut file, "openai");
        connect(&mut file, "openai");
        connect(&mut file, "ollama");
        let config = file.save().unwrap();
        let openai: Vec<_> = config
            .providers
            .iter()
            .filter(|p| p.name == "openai")
            .collect();
        assert_eq!(openai.len(), 1);
        assert_eq!(openai[0].api_key_env.as_deref(), Some("OPENAI_API_KEY"));
        assert!(config.provider("ollama").unwrap().api_key_env.is_none());
    }

    #[test]
    fn the_agent_can_differ_or_follow() {
        let (_dir, _path, mut file) = open_temp();
        connect(&mut file, "anthropic");
        file.set_quick("anthropic/claude-haiku-4-5").unwrap();
        file.set_agent(Some("anthropic/claude-sonnet-5-5")).unwrap();
        let config = file.save().unwrap();
        assert!(
            matches!(config.agent().unwrap(), AgentModel::Jig { model, .. } if model == "claude-sonnet-5-5")
        );

        file.set_agent(None).unwrap();
        let config = file.save().unwrap();
        assert!(
            matches!(config.agent().unwrap(), AgentModel::Jig { model, .. } if model == "claude-haiku-4-5")
        );
        assert!(file.set_quick("groq/llama").is_err(), "not connected");
    }

    #[test]
    fn removing_a_provider_clears_the_lanes_using_it() {
        let (_dir, _path, mut file) = open_temp();
        connect(&mut file, "anthropic");
        file.set_agent(Some("anthropic/claude-sonnet-5-5")).unwrap();
        file.remove("opencode-go").unwrap();
        let config = file.save().unwrap();
        assert!(config.quick.is_none());
        assert!(config.quick().is_err());
        assert!(config.agent.is_some(), "the agent didn't use it");

        file.remove("anthropic").unwrap();
        assert!(file.save().unwrap().providers.is_empty());
    }

    #[test]
    fn a_broken_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "quick = 3\n").unwrap();
        assert!(ConfigFile::open(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "quick = 3\n");
    }

    #[test]
    fn every_template_parses() {
        let (_dir, _path, mut file) = open_temp();
        for template in TEMPLATES {
            file.connect(template).unwrap();
        }
        file.save().unwrap();
    }
}
