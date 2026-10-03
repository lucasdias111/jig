//! OpenAI-compatible Chat Completions (`POST {base}/chat/completions`).
//! Covers Ollama, llama.cpp's server and OpenCode Go's chat models.

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::{Provider, TOOL_LIMIT_REACHED, ToolHost, http};

pub struct OpenAiCompatProvider {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub max_tokens: u32,
    /// Ask for `response_format: json_object`. Some servers reject it.
    pub json_mode: bool,
    /// Sent with every request, e.g. OpenCode Go's session header.
    pub extra_headers: Vec<(String, String)>,
    agent: ureq::Agent,
}

impl OpenAiCompatProvider {
    pub fn new(
        base_url: &str,
        api_key: Option<String>,
        model: &str,
        max_tokens: u32,
        json_mode: bool,
    ) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key,
            model: model.to_string(),
            max_tokens,
            json_mode,
            extra_headers: Vec::new(),
            agent: http::agent(),
        }
    }

    pub fn with_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.extra_headers = headers;
        self
    }

    fn send(&self, messages: &[Value], tools: Option<(&Value, bool)>) -> Result<Value> {
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "temperature": 0,
            "messages": messages,
        });
        match tools {
            Some((tools, allowed)) => {
                // JSON mode and tools don't mix on every server; the reply
                // parser copes without it.
                body["tools"] = tools.clone();
                if !allowed {
                    body["tool_choice"] = json!("none");
                }
            }
            None if self.json_mode => body["response_format"] = json!({ "type": "json_object" }),
            None => {}
        }
        let headers: Vec<(&str, String)> = self
            .api_key
            .iter()
            .map(|key| ("authorization", format!("Bearer {key}")))
            .chain(
                self.extra_headers
                    .iter()
                    .map(|(name, value)| (name.as_str(), value.clone())),
            )
            .collect();
        let url = format!("{}/chat/completions", self.base_url);
        http::post_json(&self.agent, &url, &headers, &body)
    }
}

fn text_of(response: &Value) -> Result<String> {
    response["choices"][0]["message"]["content"]
        .as_str()
        .filter(|text| !text.is_empty())
        .map(str::to_string)
        .context("The model returned no text.")
}

impl Provider for OpenAiCompatProvider {
    fn complete(&self, system: &str, user: &str) -> Result<String> {
        let messages = [
            json!({ "role": "system", "content": system }),
            json!({ "role": "user", "content": user }),
        ];
        text_of(&self.send(&messages, None)?)
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
                    "type": "function",
                    "function": {
                        "name": spec.name,
                        "description": spec.description,
                        "parameters": spec.input_schema,
                    },
                })
            })
            .collect();
        let mut messages = vec![
            json!({ "role": "system", "content": system }),
            json!({ "role": "user", "content": user }),
        ];
        let mut calls = 0;
        loop {
            let allowed = calls < max_calls;
            let response = self.send(&messages, Some((&specs, allowed)))?;
            let message = response["choices"][0]["message"].clone();
            let tool_calls = message["tool_calls"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if tool_calls.is_empty() {
                return text_of(&response);
            }
            messages.push(message);
            for call in &tool_calls {
                let name = call["function"]["name"].as_str().unwrap_or_default();
                let input: Value = call["function"]["arguments"]
                    .as_str()
                    .and_then(|arguments| serde_json::from_str(arguments).ok())
                    .unwrap_or_else(|| json!({}));
                let output = if calls < max_calls {
                    calls += 1;
                    on_step(tools.describe(name, &input));
                    tools.call(name, &input)
                } else {
                    TOOL_LIMIT_REACHED.to_string()
                };
                messages
                    .push(json!({ "role": "tool", "tool_call_id": call["id"], "content": output }));
            }
            if !allowed {
                anyhow::bail!("The model kept calling tools past the limit.");
            }
        }
    }
}
