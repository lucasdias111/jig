//! Shared HTTP plumbing for the providers.

use std::sync::Mutex;
use std::time::Duration;

use anyhow::Result;
use serde_json::Value;

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(concat!("jig/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

/// A non-2xx answer, with the provider's own message when it has one.
#[derive(Debug)]
pub struct HttpError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = ureq::http::StatusCode::from_u16(self.status)
            .ok()
            .and_then(|status| status.canonical_reason())
            .unwrap_or("error");
        write!(f, "{} {reason}: {}", self.status, self.message)
    }
}

impl std::error::Error for HttpError {}

/// POST `body` as JSON and return the JSON response. A non-2xx status
/// becomes an [`HttpError`].
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
    let response = request
        .send_json(body)
        .map_err(|error| anyhow::anyhow!("Couldn't reach {url}: {error}"))?;
    read_json(url, response)
}

/// GET `url` and return the JSON response.
pub fn get_json(agent: &ureq::Agent, url: &str, headers: &[(&str, String)]) -> Result<Value> {
    let mut request = agent.get(url);
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let response = request
        .call()
        .map_err(|error| anyhow::anyhow!("Couldn't reach {url}: {error}"))?;
    read_json(url, response)
}

fn read_json(url: &str, mut response: ureq::http::Response<ureq::Body>) -> Result<Value> {
    let status = response.status();
    let text = response.body_mut().read_to_string()?;
    if !status.is_success() {
        let message = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|json| error_message(&json))
            .unwrap_or_else(|| text.chars().take(200).collect());
        return Err(HttpError {
            status: status.as_u16(),
            message,
        }
        .into());
    }
    serde_json::from_str(&text).map_err(|error| anyhow::anyhow!("Invalid JSON from {url}: {error}"))
}

/// A change to the request body for servers that reject part of it.
#[derive(Clone, Copy, Debug)]
pub enum Fallback {
    /// Leave the field out.
    Drop(&'static str),
    /// Send the field under another name, as OpenAI's reasoning models want
    /// `max_completion_tokens` for `max_tokens`.
    Rename(&'static str, &'static str),
}

impl Fallback {
    fn field(self) -> &'static str {
        match self {
            Fallback::Drop(field) | Fallback::Rename(field, _) => field,
        }
    }

    fn apply(self, body: &mut Value) {
        let Some(object) = body.as_object_mut() else {
            return;
        };
        let removed = object.remove(self.field());
        if let (Fallback::Rename(_, to), Some(removed)) = (self, removed) {
            object.insert(to.to_string(), removed);
        }
    }
}

/// Sends requests with fields not every server accepts, such as
/// `response_format` or `reasoning_effort`. When a server rejects one by
/// name, it's dropped and the request retried, and it stays dropped for the
/// rest of this provider's life.
pub struct Lenient {
    fallbacks: &'static [Fallback],
    applied: Mutex<Vec<usize>>,
}

impl Lenient {
    pub fn new(fallbacks: &'static [Fallback]) -> Self {
        Self {
            fallbacks,
            applied: Mutex::new(Vec::new()),
        }
    }

    pub fn post_json(
        &self,
        agent: &ureq::Agent,
        url: &str,
        headers: &[(&str, String)],
        mut body: Value,
    ) -> Result<Value> {
        let mut applied = self
            .applied
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        for &index in &applied {
            self.fallbacks[index].apply(&mut body);
        }
        loop {
            let error = match post_json(agent, url, headers, &body) {
                Ok(response) => return Ok(response),
                Err(error) => error,
            };
            let Some(index) = self.rejected(&error, &body) else {
                return Err(error);
            };
            self.fallbacks[index].apply(&mut body);
            applied.push(index);
            *self
                .applied
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = applied.clone();
        }
    }

    /// The fallback for a field `error` names, if the body still has it.
    fn rejected(&self, error: &anyhow::Error, body: &Value) -> Option<usize> {
        let error = error.downcast_ref::<HttpError>()?;
        if !(400..500).contains(&error.status) || error.status == 401 || error.status == 403 {
            return None;
        }
        self.fallbacks.iter().position(|fallback| {
            body.get(fallback.field()).is_some() && error.message.contains(fallback.field())
        })
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const FALLBACKS: &[Fallback] = &[
        Fallback::Drop("reasoning_effort"),
        Fallback::Rename("max_tokens", "max_completion_tokens"),
    ];

    fn rejection(status: u16, message: &str) -> anyhow::Error {
        HttpError {
            status,
            message: message.into(),
        }
        .into()
    }

    #[test]
    fn only_named_fields_are_dropped() {
        let lenient = Lenient::new(FALLBACKS);
        let body = json!({"reasoning_effort": "none", "max_tokens": 10});
        let named = rejection(
            400,
            "Unrecognized request argument supplied: reasoning_effort",
        );
        assert_eq!(lenient.rejected(&named, &body), Some(0));
        let vague = rejection(400, "Invalid request");
        assert_eq!(lenient.rejected(&vague, &body), None);
        let auth = rejection(401, "bad key for reasoning_effort");
        assert_eq!(lenient.rejected(&auth, &body), None);
    }

    #[test]
    fn rename_keeps_the_value() {
        let mut body = json!({"max_tokens": 10});
        FALLBACKS[1].apply(&mut body);
        assert_eq!(body, json!({"max_completion_tokens": 10}));
    }
}
