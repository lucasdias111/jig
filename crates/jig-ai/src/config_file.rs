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
    /// Words in the names of its small, fast models, best first: what the
    /// agent starts on when this is the first provider connected.
    pub fast: &'static [&'static str],
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
        fast: &[],
        label,
        name,
        kind,
        base_url,
        api_key_env,
        session_header: None,
    }
}

impl ProviderTemplate {
    /// The model the agent starts on, from the ones it offers: the first
    /// with the best of its `fast` words, else the first.
    pub fn suggested_model(&self, models: &[String]) -> Option<String> {
        self.fast
            .iter()
            .find_map(|word| {
                models
                    .iter()
                    .find(|model| model.to_lowercase().contains(word))
            })
            .or_else(|| models.first())
            .cloned()
    }
}

/// The providers Settings lists. Almost every provider speaks one of the
/// two formats, so these differ only in where they point.
pub const TEMPLATES: &[ProviderTemplate] = &[
    ProviderTemplate {
        fast: &["haiku"],
        ..template(
            "Anthropic",
            "anthropic",
            ProviderKind::Anthropic,
            "https://api.anthropic.com/v1",
            Some("ANTHROPIC_API_KEY"),
        )
    },
    ProviderTemplate {
        fast: &["mini", "nano"],
        ..template(
            "OpenAI",
            "openai",
            ProviderKind::Openai,
            "https://api.openai.com/v1",
            Some("OPENAI_API_KEY"),
        )
    },
    ProviderTemplate {
        fast: &["flash"],
        ..template(
            "Google Gemini",
            "gemini",
            ProviderKind::Openai,
            "https://generativelanguage.googleapis.com/v1beta/openai",
            Some("GEMINI_API_KEY"),
        )
    },
    ProviderTemplate {
        fast: &["haiku", "flash", "mini"],
        ..template(
            "OpenRouter",
            "openrouter",
            ProviderKind::Openai,
            "https://openrouter.ai/api/v1",
            Some("OPENROUTER_API_KEY"),
        )
    },
    ProviderTemplate {
        fast: &["instant", "8b"],
        ..template(
            "Groq",
            "groq",
            ProviderKind::Openai,
            "https://api.groq.com/openai/v1",
            Some("GROQ_API_KEY"),
        )
    },
    ProviderTemplate {
        fast: &["small"],
        ..template(
            "Mistral",
            "mistral",
            ProviderKind::Openai,
            "https://api.mistral.ai/v1",
            Some("MISTRAL_API_KEY"),
        )
    },
    ProviderTemplate {
        fast: &["chat"],
        ..template(
            "DeepSeek",
            "deepseek",
            ProviderKind::Openai,
            "https://api.deepseek.com/v1",
            Some("DEEPSEEK_API_KEY"),
        )
    },
    ProviderTemplate {
        fast: &["flash"],
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
    // `llama-server` from llama.cpp, on its default port.
    template(
        "llama.cpp",
        "llama-cpp",
        ProviderKind::Openai,
        "http://localhost:8080/v1",
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

    /// The `[[provider]]` tables, started when there are none yet, as in
    /// a file before the first provider is connected.
    fn providers(&mut self) -> Result<&mut ArrayOfTables> {
        if !self.doc.contains_key("provider") {
            self.doc.insert(
                "provider",
                toml_edit::Item::ArrayOfTables(ArrayOfTables::new()),
            );
        }
        self.doc
            .get_mut("provider")
            .and_then(|item| item.as_array_of_tables_mut())
            .context("config.toml's `provider` should be [[provider]] tables")
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

    /// Set the model the agent uses, as `provider/model`.
    pub fn set_agent(&mut self, id: &str) -> Result<()> {
        self.check_model(id)?;
        self.doc["agent"] = value(id);
        // Older files' ways of saying it, and Jig mode's retired model,
        // would only confuse.
        for retired in ["agent_model", "quick", "default"] {
            self.doc.remove(retired);
        }
        Ok(())
    }

    fn check_model(&mut self, id: &str) -> Result<()> {
        let (name, _) = split_model(id)
            .with_context(|| format!("\"{id}\" should look like provider/model."))?;
        self.provider(name)?;
        Ok(())
    }

    /// Remove a provider. An agent that used it is left without a model, so
    /// Settings asks for another.
    pub fn remove(&mut self, name: &str) -> Result<()> {
        let providers = self.providers()?;
        let index = providers
            .iter()
            .position(|table| table.get("name").and_then(|item| item.as_str()) == Some(name))
            .with_context(|| format!("There's no provider called \"{name}\"."))?;
        providers.remove(index);
        let prefix = format!("{name}/");
        if self
            .doc
            .get("agent")
            .and_then(|item| item.as_str())
            .is_some_and(|id| id.starts_with(&prefix))
        {
            self.doc.remove("agent");
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
    fn a_first_provider_suggests_a_fast_model() {
        let models = |names: &[&str]| {
            names
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>()
        };
        let anthropic = template_named("anthropic").unwrap();
        assert_eq!(
            anthropic
                .suggested_model(&models(&["claude-opus-5-5", "claude-haiku-5-5"]))
                .as_deref(),
            Some("claude-haiku-5-5")
        );
        let ollama = template_named("ollama").unwrap();
        assert_eq!(
            ollama.suggested_model(&models(&["qwen3:8b"])).as_deref(),
            Some("qwen3:8b")
        );
        assert_eq!(ollama.suggested_model(&[]), None);
    }

    #[test]
    fn edits_keep_the_users_comments() {
        let (_dir, path, mut file) = open_temp();
        connect(&mut file, "anthropic");
        file.set_agent("anthropic/claude-haiku-4-5").unwrap();
        let config = file.save().unwrap();
        assert!(
            matches!(config.agent().unwrap(), AgentModel::Jig { model, .. } if model == "claude-haiku-4-5")
        );
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
    fn picking_the_agents_model_drops_the_retired_ones() {
        let (_dir, path, mut file) = open_temp();
        connect(&mut file, "anthropic");
        std::fs::write(
            &path,
            format!("quick = \"anthropic/claude-haiku-4-5\"\n{}", file.doc),
        )
        .unwrap();
        let mut file = ConfigFile::open(&path).unwrap();
        file.set_agent("anthropic/claude-sonnet-5-5").unwrap();
        file.save().unwrap();
        let saved = std::fs::read_to_string(&path).unwrap();
        assert!(!saved.contains("quick"), "{saved}");
        assert!(file.set_agent("groq/llama").is_err(), "not connected");
    }

    #[test]
    fn removing_a_provider_clears_the_agents_model_if_it_used_it() {
        let (_dir, _path, mut file) = open_temp();
        connect(&mut file, "anthropic");
        connect(&mut file, "groq");
        file.set_agent("anthropic/claude-sonnet-5-5").unwrap();
        file.remove("groq").unwrap();
        let config = file.save().unwrap();
        assert!(config.agent.is_some(), "the agent didn't use it");

        file.remove("anthropic").unwrap();
        let config = file.save().unwrap();
        assert!(config.agent.is_none());
        assert!(config.providers.is_empty());
    }

    #[test]
    fn a_broken_file_is_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "agent = 3\n").unwrap();
        assert!(ConfigFile::open(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "agent = 3\n");
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
