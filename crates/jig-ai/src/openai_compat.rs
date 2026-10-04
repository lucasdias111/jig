//! OpenAI-compatible Chat Completions (`POST {base}/chat/completions`).
//! Covers OpenAI, Gemini, OpenRouter, Groq, Mistral, DeepSeek, Ollama,
//! LM Studio, llama.cpp's server and OpenCode Go's chat models.

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::http::{Fallback, Lenient};
use crate::{Provider, http};

/// What servers that claim OpenAI compatibility still reject: JSON mode,
/// the reasoning switch, temperature on reasoning models, and `max_tokens`,
/// which OpenAI's reasoning models want as `max_completion_tokens`.
const FALLBACKS: &[Fallback] = &[
    Fallback::Drop("response_format"),
    Fallback::Drop("reasoning_effort"),
    Fallback::Drop("temperature"),
    Fallback::Rename("max_tokens", "max_completion_tokens"),
];

pub struct OpenAiCompatProvider {
    pub base_url: String,
    pub api_key: Option<String>,
    pub model: String,
    pub max_tokens: u32,
    /// Ask for `response_format: json_object`. Some servers reject it.
    pub json_mode: bool,
    /// Let the model reason before it answers. Off by default: reasoning
    /// models think for 10-60 s even on a one-line edit, and Jig's commands
    /// are small enough not to need it.
    pub thinking: bool,
    /// Sent with every request, e.g. OpenCode Go's session header.
    pub extra_headers: Vec<(String, String)>,
    agent: ureq::Agent,
    lenient: Lenient,
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
            thinking: false,
            extra_headers: Vec::new(),
            agent: http::agent(),
            lenient: Lenient::new(FALLBACKS),
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

    fn send(&self, messages: &[Value]) -> Result<Value> {
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "temperature": 0,
            "messages": messages,
        });
        if self.json_mode {
            body["response_format"] = json!({ "type": "json_object" });
        }
        if !self.thinking {
            body["reasoning_effort"] = json!("none");
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
        self.lenient.post_json(&self.agent, &url, &headers, body)
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
        text_of(&self.send(&messages)?)
    }
}
