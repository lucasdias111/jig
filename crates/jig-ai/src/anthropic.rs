//! Anthropic Messages API (`POST {base}/messages`). Also serves OpenCode Go
//! models that speak the Anthropic format.

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::Provider;
use crate::config::AuthStyle;
use crate::http;

pub struct AnthropicProvider {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub auth: AuthStyle,
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
            agent: http::agent(),
        }
    }
}

impl Provider for AnthropicProvider {
    fn complete(&self, system: &str, user: &str) -> Result<String> {
        let body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "system": system,
            "messages": [{ "role": "user", "content": user }],
        });
        let auth = match self.auth {
            AuthStyle::ApiKey => ("x-api-key", self.api_key.clone()),
            AuthStyle::Bearer => ("authorization", format!("Bearer {}", self.api_key)),
        };
        let headers = [auth, ("anthropic-version", "2023-06-01".to_string())];
        let response = http::post_json(
            &self.agent,
            &format!("{}/messages", self.base_url),
            &headers,
            &body,
        )?;
        text_of(&response)
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
