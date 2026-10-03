//! Shared HTTP plumbing for the providers.

use std::time::Duration;

use anyhow::{Result, bail};
use serde_json::Value;

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(concat!("jig/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// POST `body` as JSON and return the JSON response. A non-2xx status
/// becomes an error carrying the provider's own message when it has one.
pub fn post_json(
    agent: &ureq::Agent,
    url: &str,
    headers: &[(&str, String)],
    body: &Value,
) -> Result<Value> {
    let mut request = agent.post(url);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let mut response = request
        .send_json(body)
        .map_err(|error| anyhow::anyhow!("Couldn't reach {url}: {error}"))?;
    let status = response.status();
    let text = response.body_mut().read_to_string()?;
    if !status.is_success() {
        let detail = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|json| error_message(&json))
            .unwrap_or_else(|| text.chars().take(200).collect());
        bail!(
            "{} {}: {detail}",
            status.as_u16(),
            status.canonical_reason().unwrap_or("error")
        );
    }
    serde_json::from_str(&text).map_err(|error| anyhow::anyhow!("Invalid JSON from {url}: {error}"))
}

/// `{"error": {"message": ...}}` (OpenAI, Anthropic) or `{"error": "..."}`.
fn error_message(json: &Value) -> Option<String> {
    let error = json.get("error")?;
    error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| error.as_str())
        .map(str::to_string)
}
