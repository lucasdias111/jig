//! OpenAI-compatible Chat Completions (`POST {base}/chat/completions`).
//! Covers OpenAI, Gemini, OpenRouter, Groq, Mistral, DeepSeek, Ollama,
//! LM Studio, llama.cpp's server and OpenCode Go's chat models.

use anyhow::{Context as _, Result};
use serde_json::{Value, json};

use crate::http::{Fallback, Lenient};
use crate::{Provider, http};

/// What servers that claim OpenAI compatibility still reject: JSON mode,
/// the reasoning switches, temperature on reasoning models, and
/// `max_tokens`, which OpenAI's reasoning models want as
/// `max_completion_tokens`.
const FALLBACKS: &[Fallback] = &[
    Fallback::Drop("response_format"),
    Fallback::Drop("reasoning_effort"),
    Fallback::Drop("chat_template_kwargs"),
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

    fn body(&self, messages: &[Value]) -> Value {
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
            // Servers on this computer (llama.cpp, LM Studio, vLLM) run the
            // model's own chat template, whose switch this is; Qwen3 and
            // the like think by default otherwise. Cloud APIs don't need it.
            if self.is_local() {
                body["chat_template_kwargs"] = json!({ "enable_thinking": false });
            }
        }
        body
    }

    /// The server runs on this computer.
    fn is_local(&self) -> bool {
        let address = self.base_url.split("://").nth(1).unwrap_or(&self.base_url);
        ["localhost", "127.0.0.1", "[::1]", "0.0.0.0"]
            .iter()
            .any(|host| {
                address
                    .strip_prefix(host)
                    .is_some_and(|rest| rest.is_empty() || rest.starts_with([':', '/']))
            })
    }

    fn send(&self, messages: &[Value]) -> Result<Value> {
        let body = self.body(messages);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn provider(base_url: &str) -> OpenAiCompatProvider {
        OpenAiCompatProvider::new(base_url, None, "m", 100, true)
    }

    #[test]
    fn local_servers_are_told_not_to_think_by_their_chat_template() {
        let local = provider("http://localhost:8080/v1").body(&[]);
        assert_eq!(local["chat_template_kwargs"]["enable_thinking"], false);
        assert_eq!(local["reasoning_effort"], "none");
        for url in ["http://127.0.0.1:1234/v1", "http://[::1]:8080/v1"] {
            assert!(
                provider(url).body(&[])["chat_template_kwargs"].is_object(),
                "{url}"
            );
        }
        let cloud = provider("https://api.openai.com/v1").body(&[]);
        assert!(cloud.get("chat_template_kwargs").is_none());
        assert!(
            provider("https://localhost.example.com/v1").body(&[])["chat_template_kwargs"]
                .is_null(),
            "not fooled by the name"
        );
        let thinking = provider("http://localhost:8080/v1")
            .with_thinking(true)
            .body(&[]);
        assert!(
            thinking.get("chat_template_kwargs").is_none(),
            "left to think when asked"
        );
    }
}
