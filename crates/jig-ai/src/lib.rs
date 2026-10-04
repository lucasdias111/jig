//! The AI layer: a `Provider` trait, prompt building and response parsing.
//!
//! Calls are blocking; the app runs them on a background thread.

pub mod agent;
mod anthropic;
pub mod config;
mod http;
mod openai_compat;
pub mod prompt;
pub mod response;

pub use anthropic::AnthropicProvider;
pub use config::{Config, ProviderConfig, ProviderKind};
pub use openai_compat::OpenAiCompatProvider;
pub use prompt::PromptRequest;
pub use response::Reply;

/// A model endpoint. `complete` sends one system + user message pair and
/// returns the model's raw text.
pub trait Provider: Send + Sync {
    fn complete(&self, system: &str, user: &str) -> anyhow::Result<String>;
}

/// Build the prompt, call the provider and parse the reply.
pub fn run(provider: &dyn Provider, request: &PromptRequest) -> anyhow::Result<Reply> {
    let (system, user) = prompt::build(request);
    let raw = provider.complete(&system, &user)?;
    response::parse(&raw, request.target_text())
}
