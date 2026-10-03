//! Anthropic Messages API (`POST {base}/messages`). Also serves OpenCode Go
//! models that speak the Anthropic format.

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::config::AuthStyle;
use crate::{Provider, TOOL_LIMIT_REACHED, ToolHost, http};

pub struct AnthropicProvider {
    pub base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
    pub auth: AuthStyle,
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
            extra_headers: Vec::new(),
            agent: http::agent(),
        }
    }

    pub fn with_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.extra_headers = headers;
        self
    }

    fn send(
        &self,
        system: &str,
        messages: &[Value],
        tools: Option<(&Value, bool)>,
    ) -> Result<Value> {
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "system": system,
            "messages": messages,
        });
        if let Some((tools, allowed)) = tools {
            body["tools"] = tools.clone();
            if !allowed {
                body["tool_choice"] = json!({ "type": "none" });
            }
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
    fn complete(&self, system: &str, user: &str) -> Result<String> {
        let response = self.send(system, &[json!({ "role": "user", "content": user })], None)?;
        text_of(&response)
    }

    fn complete_with_tools(
        &self,
        system: &str,
        user: &str,
        tools: &dyn ToolHost,
        max_calls: usize,
        on_step: &dyn Fn(String),
    ) -> Result<String> {
        let specs: Value = tools
            .specs()
            .into_iter()
            .map(|spec| {
                json!({
                    "name": spec.name,
                    "description": spec.description,
                    "input_schema": spec.input_schema,
                })
            })
            .collect();
        let mut messages = vec![json!({ "role": "user", "content": user })];
        let mut calls = 0;
        loop {
            let allowed = calls < max_calls;
            let response = self.send(system, &messages, Some((&specs, allowed)))?;
            let content = response["content"].as_array().cloned().unwrap_or_default();
            let uses: Vec<&Value> = content
                .iter()
                .filter(|block| block["type"] == "tool_use")
                .collect();
            if uses.is_empty() {
                return text_of(&response);
            }
            let results: Vec<Value> = uses
                .iter()
                .map(|block| {
                    let (name, input) =
                        (block["name"].as_str().unwrap_or_default(), &block["input"]);
                    let output = if calls < max_calls {
                        calls += 1;
                        on_step(tools.describe(name, input));
                        tools.call(name, input)
                    } else {
                        TOOL_LIMIT_REACHED.to_string()
                    };
                    json!({ "type": "tool_result", "tool_use_id": block["id"], "content": output })
                })
                .collect();
            messages.push(json!({ "role": "assistant", "content": content }));
            messages.push(json!({ "role": "user", "content": results }));
            if !allowed {
                // The model ignored tool_choice: none. Don't loop forever.
                anyhow::bail!("The model kept calling tools past the limit.");
            }
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
