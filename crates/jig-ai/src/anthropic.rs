//! Anthropic Messages API (`POST {base}/messages`). Also serves OpenCode Go
//! models that speak the Anthropic format.

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::config::AuthStyle;
use crate::{Provider, http};

pub struct AnthropicProvider {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub auth: AuthStyle,
    /// Let the model reason before it answers. Off by default: reasoning
    /// models think for 10-60 s even on a one-line edit, and Jig's commands
    /// are small enough not to need it.
    pub thinking: bool,
    /// Sent with every request, e.g. OpenCode Go's session header.
    pub extra_headers: Vec<(String, String)>,
    agent: ureq::Agent,
}

impl AnthropicProvider {
    pub fn new(
        base_url: &str,
        api_key: String,
        model: &str,
        max_tokens: u32,
        auth: AuthStyle,
    ) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model: model.to_string(),
            max_tokens,
            auth,
            thinking: false,
            extra_headers: Vec::new(),
            agent: http::agent(),
        }
    }

    pub fn with_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.extra_headers = headers;
        self
    }

    pub fn with_thinking(mut self, thinking: bool) -> Self {
        self.thinking = thinking;
        self
    }

    fn send(&self, system: &str, messages: &[Value]) -> Result<Value> {
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "system": system,
            "messages": messages,
        });
        body["tools"] = json!([{
            "name": crate::response::REPLY_TOOL,
            "description": "Apply the edit to the marked region.",
            "input_schema": crate::response::reply_schema(),
        }]);
        if self.thinking {
            // A forced tool call isn't allowed while thinking.
            body["tool_choice"] = json!({ "type": "auto" });
        } else {
            body["thinking"] = json!({ "type": "disabled" });
            body["tool_choice"] = json!({ "type": "tool", "name": crate::response::REPLY_TOOL });
        }
        let auth = match self.auth {
            AuthStyle::ApiKey => ("x-api-key", self.api_key.clone()),
            AuthStyle::Bearer => ("authorization", format!("Bearer {}", self.api_key)),
        };
        let mut headers = vec![auth, ("anthropic-version", "2023-06-01".to_string())];
        headers.extend(
            self.extra_headers
                .iter()
                .map(|(name, value)| (name.as_str(), value.clone())),
        );
        http::post_json(
            &self.agent,
            &format!("{}/messages", self.base_url),
            &headers,
            &body,
        )
    }
}

impl Provider for AnthropicProvider {
    /// The reply as JSON text: the input of the `apply_edit` call, or the
    /// model's text if it answered without one.
    fn complete(&self, system: &str, user: &str) -> Result<String> {
        let response = self.send(system, &[json!({ "role": "user", "content": user })])?;
        let call = response["content"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|block| {
                block["type"] == "tool_use" && block["name"] == crate::response::REPLY_TOOL
            });
        match call {
            Some(call) => Ok(call["input"].to_string()),
            None => text_of(&response),
        }
    }
}

/// Concatenate the text blocks of a Messages API response.
fn text_of(response: &Value) -> Result<String> {
    let blocks = response["content"]
        .as_array()
        .context("Response has no content.")?;
    let text: String = blocks
        .iter()
        .filter(|block| block["type"] == "text")
        .filter_map(|block| block["text"].as_str())
        .collect();
    if text.is_empty() {
        anyhow::bail!("The model returned no text.");
    }
    Ok(text)
}
