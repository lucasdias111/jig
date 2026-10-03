//! The AI layer: a `Provider` trait, prompt building and response parsing.
//!
//! Calls are blocking; the app runs them on a background thread.

mod anthropic;
pub mod config;
mod http;
mod openai_compat;
pub mod prompt;
pub mod response;
pub mod tools;

pub use anthropic::AnthropicProvider;
pub use config::{Config, ProviderConfig, ProviderKind};
pub use openai_compat::OpenAiCompatProvider;
pub use prompt::PromptRequest;
pub use response::Reply;
pub use tools::{ProjectTools, ToolHost, ToolSpec};

/// A model endpoint. `complete` sends one system + user message pair and
/// returns the model's raw text.
pub trait Provider: Send + Sync {
    fn complete(&self, system: &str, user: &str) -> anyhow::Result<String>;

    /// Like [`Provider::complete`], but the model may first call `tools`, at
    /// most `max_calls` times in total. `on_step` gets a short description of
    /// each call as it starts. Providers without tool support just complete.
    fn complete_with_tools(
        &self,
        system: &str,
        user: &str,
        tools: &dyn ToolHost,
        max_calls: usize,
        on_step: &dyn Fn(String),
    ) -> anyhow::Result<String> {
        let _ = (tools, max_calls, on_step);
        self.complete(system, user)
    }
}

/// Most tool calls one exploring command may make.
pub const MAX_TOOL_CALLS: usize = 8;

/// What a tool call returns once the budget is spent.
pub(crate) const TOOL_LIMIT_REACHED: &str =
    "Tool limit reached. Do not call more tools; reply with the JSON object now.";

/// Build the prompt, call the provider and parse the reply.
pub fn run(provider: &dyn Provider, request: &PromptRequest) -> anyhow::Result<Reply> {
    let (system, user) = prompt::build(request);
    let raw = provider.complete(&system, &user)?;
    response::parse(&raw, request.target_text())
}

/// Like [`run`], but the model may explore the project with `tools` first.
pub fn run_exploring(
    provider: &dyn Provider,
    request: &PromptRequest,
    tools: &dyn ToolHost,
    on_step: &dyn Fn(String),
) -> anyhow::Result<Reply> {
    let (system, user) = prompt::build(request);
    let system = format!("{system}\n\n{}", prompt::EXPLORE);
    let raw = provider.complete_with_tools(&system, &user, tools, MAX_TOOL_CALLS, on_step)?;
    response::parse(&raw, request.target_text())
}
