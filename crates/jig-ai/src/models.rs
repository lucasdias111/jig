//! Asking a provider which models it has, for the Model list in Settings.

use anyhow::{Context as _, Result};
use serde_json::Value;

use crate::config::{AuthStyle, ProviderConfig, ProviderKind, session_id};
use crate::http;

impl ProviderConfig {
    /// The model IDs the provider offers, from `GET {base_url}/models`,
    /// which Anthropic, OpenAI-compatible servers and Ollama all answer.
    pub fn list_models(&self, api_key: Option<&str>) -> Result<Vec<String>> {
        let mut headers: Vec<(&str, String)> = Vec::new();
        if let Some(key) = api_key {
            headers.push(match (self.kind, self.auth) {
                (ProviderKind::Anthropic, AuthStyle::ApiKey) => ("x-api-key", key.to_string()),
                _ => ("authorization", format!("Bearer {key}")),
            });
        }
        if self.kind == ProviderKind::Anthropic {
            headers.push(("anthropic-version", "2023-06-01".to_string()));
        }
        if let Some(header) = &self.session_header {
            headers.push((header, session_id().to_string()));
        }
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        model_ids(&http::get_json(&http::agent(), &url, &headers)?)
    }
}

/// Words in the IDs of models that can't edit code: embeddings, speech,
/// images and moderation, which OpenAI and OpenRouter list beside chat
/// models.
const NOT_FOR_CODE: &[&str] = &[
    "embed",
    "tts",
    "whisper",
    "transcribe",
    "audio",
    "realtime",
    "dall-e",
    "image",
    "moderation",
    "rerank",
    "guard",
];

/// `{"data": [{"id": ...}, ...]}`: the models that can write code, sorted.
fn model_ids(response: &Value) -> Result<Vec<String>> {
    let mut ids: Vec<String> = response["data"]
        .as_array()
        .context("The provider didn't list its models.")?
        .iter()
        .filter_map(|model| model["id"].as_str().map(str::to_string))
        .filter(|id| {
            let id = id.to_lowercase();
            !NOT_FOR_CODE.iter().any(|word| id.contains(word))
        })
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_ids_in_order() {
        let response = json!({"data": [{"id": "b"}, {"id": "a", "type": "model"}, {"x": 1}]});
        assert_eq!(model_ids(&response).unwrap(), ["a", "b"]);
        assert!(model_ids(&json!({"models": []})).is_err());
        let mixed = json!({"data": [
            {"id": "gpt-5"}, {"id": "text-embedding-3-small"}, {"id": "gpt-4o-mini-tts"},
            {"id": "whisper-1"}, {"id": "omni-moderation-latest"}, {"id": "gpt-image-1"},
        ]});
        assert_eq!(model_ids(&mixed).unwrap(), ["gpt-5"]);
    }
}
