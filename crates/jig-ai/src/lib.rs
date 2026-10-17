//! The AI layer: a `Provider` trait, prompt building and response parsing.
//!
//! Calls are blocking; the app runs them on a background thread.

pub mod agent;
mod anthropic;
pub mod config;
pub mod config_file;
mod http;
pub mod keys;
mod models;
mod openai_compat;
pub mod prompt;
pub mod response;
pub mod tasks;

pub use anthropic::AnthropicProvider;
pub use config::{AgentModel, Config, ProviderConfig, ProviderKind, split_model};
pub use config_file::{ConfigFile, ProviderTemplate, TEMPLATES, template_named};
pub use keys::{KeySource, KeyStore};
pub use openai_compat::OpenAiCompatProvider;
pub use prompt::PromptRequest;
pub use response::{Reply, ReplyKind};

/// A model endpoint. `complete` sends one system + user message pair and
/// returns the model's raw text.
pub trait Provider: Send + Sync {
    fn complete(&self, system: &str, user: &str) -> anyhow::Result<String>;

    /// As [`Self::complete`], for a reply of `kind`. Providers that force
    /// the reply through a tool pick the tool for it; the others are told
    /// the shape in the prompt alone.
    fn complete_as(&self, system: &str, user: &str, kind: ReplyKind) -> anyhow::Result<String> {
        let _ = kind;
        self.complete(system, user)
    }
}

/// Build the prompt, call the provider and parse the reply.
pub fn run(provider: &dyn Provider, request: &PromptRequest) -> anyhow::Result<Reply> {
    request.check_target()?;
    let (system, user) = prompt::build(request);
    if request.answer {
        let raw = provider.complete_as(&system, &user, ReplyKind::Answer)?;
        return response::parse_answer(&raw, request.target_text());
    }
    let raw = provider.complete_as(&system, &user, ReplyKind::Edit)?;
    response::parse(&raw, request.target_text())
}

/// Send one small command and check the reply is in Jig's format, so
/// Settings can say whether a provider works before it's used.
pub fn check(provider: &dyn Provider) -> anyhow::Result<Reply> {
    let text = "fn area(w: f64, h: f64) -> f64 {\n    w * h\n}\n";
    let request = PromptRequest {
        task: None,
        instruction: "Rename the parameters to width and height.".into(),
        comment: None,
        project_rules: None,
        language: "rust".into(),
        file_name: Some("area.rs".into()),
        text: text.into(),
        target: 0..text.len(),
        diagnostics: Vec::new(),
        answer: false,
    };
    run(provider, &request)
}
