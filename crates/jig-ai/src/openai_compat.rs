//! OpenAI-compatible Chat Completions (`POST {base}/chat/completions`).
//! Covers Ollama, llama.cpp's server and OpenCode Go's chat models.

use anyhow::{Context as _, Result};
use serde_json::json;

use crate::Provider;
use crate::http;

pub struct OpenAiCompatProvider {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub max_tokens: u32,
    /// Ask for `response_format: json_object`. Some servers reject it.
    pub json_mode: bool,
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
            agent: http::agent(),
        }
    }
}

impl Provider for OpenAiCompatProvider {
    fn complete(&self, system: &str, user: &str) -> Result<String> {
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "temperature": 0,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user },
            ],
        });
        if self.json_mode {
            body["response_format"] = json!({ "type": "json_object" });
        }
        let headers: Vec<(&str, String)> = self
            .api_key
            .iter()
            .map(|key| ("authorization", format!("Bearer {key}")))
            .collect();
        let url = format!("{}/chat/completions", self.base_url);
        let response = http::post_json(&self.agent, &url, &headers, &body)?;
        response["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .context("The model returned no text.")
    }
}
