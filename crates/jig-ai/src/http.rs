//! HTTP for asking providers which models they have.

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

/// GET `url` and return the JSON response.
pub fn get_json(agent: &ureq::Agent, url: &str, headers: &[(&str, String)]) -> Result<Value> {
    with_retries(url, || {
        let mut request = agent.get(url);
        for (name, value) in headers {
            request = request.header(*name, value);
        }
        request.call()
    })
}

/// How many more times a request is tried after a busy server or a failed
/// connection, and the wait before the first; each wait after is three
/// times longer. Few and short, since Settings waits on the answer.
const RETRIES: u32 = 2;
const FIRST_WAIT: Duration = Duration::from_millis(500);
/// A server asking for a longer wait than this is answered with its error.
const LONGEST_WAIT: Duration = Duration::from_secs(10);

type Response = ureq::http::Response<ureq::Body>;

/// `send`, tried again while it fails in a way that may pass.
fn with_retries(url: &str, send: impl Fn() -> Result<Response, ureq::Error>) -> Result<Value> {
    let mut attempt = 0;
    loop {
        let result = send();
        let asked = match &result {
            Ok(response) if busy(response.status().as_u16()) => Some(retry_after(response)),
            Err(error) if transient(error) => Some(None),
            _ => None,
        };
        if let Some(wait) = asked.and_then(|asked| backoff(attempt, asked)) {
            std::thread::sleep(wait);
            attempt += 1;
            continue;
        }
        let response = result.map_err(|error| anyhow::anyhow!("Couldn't reach {url}: {error}"))?;
        return read_json(url, response);
    }
}

/// Statuses that say "not now" rather than "no": rate limits, overload
/// (Anthropic's 529) and a gateway that lost the server.
fn busy(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504 | 529)
}

/// Failures before the request reached the server. A request that ran out
/// of its whole time isn't tried again: that would only double the wait.
fn transient(error: &ureq::Error) -> bool {
    use std::io::ErrorKind;
    match error {
        ureq::Error::Timeout(timeout) => {
            matches!(timeout, ureq::Timeout::Resolve | ureq::Timeout::Connect)
        }
        ureq::Error::ConnectionFailed => true,
        ureq::Error::Io(error) => matches!(
            error.kind(),
            ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted | ErrorKind::BrokenPipe
        ),
        _ => false,
    }
}

/// `Retry-After` in seconds; a date isn't worth parsing for waits this short.
fn retry_after(response: &Response) -> Option<Duration> {
    let seconds = response.headers().get("retry-after")?.to_str().ok()?;
    seconds.trim().parse().ok().map(Duration::from_secs)
}

/// The wait before trying again after `attempt` failed tries less one, or
/// `None` to give up.
fn backoff(attempt: u32, asked: Option<Duration>) -> Option<Duration> {
    if attempt >= RETRIES {
        return None;
    }
    let wait = asked.unwrap_or(FIRST_WAIT * 3u32.pow(attempt));
    (wait <= LONGEST_WAIT).then_some(wait)
}

fn read_json(url: &str, mut response: Response) -> Result<Value> {
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

    #[test]
    fn busy_servers_and_lost_connections_are_tried_again_briefly() {
        assert!(busy(429) && busy(503) && busy(529));
        assert!(!busy(400) && !busy(401) && !busy(404));
        assert!(transient(&ureq::Error::ConnectionFailed));
        assert!(transient(&ureq::Error::Timeout(ureq::Timeout::Connect)));
        assert!(!transient(&ureq::Error::Timeout(ureq::Timeout::Global)));
        assert!(!transient(&ureq::Error::HostNotFound));

        assert_eq!(backoff(0, None), Some(Duration::from_millis(500)));
        assert_eq!(backoff(1, None), Some(Duration::from_millis(1500)));
        assert_eq!(backoff(2, None), None, "gives up after two retries");
        assert_eq!(
            backoff(0, Some(Duration::from_secs(3))),
            Some(Duration::from_secs(3))
        );
        assert_eq!(backoff(0, Some(Duration::from_secs(60))), None);
    }

    #[test]
    fn a_busy_server_is_asked_again_until_it_answers() {
        use std::cell::Cell;
        let tries = Cell::new(0);
        let answer = with_retries("http://test", || {
            tries.set(tries.get() + 1);
            let status = if tries.get() < 3 { 503 } else { 200 };
            Ok(ureq::http::Response::builder()
                .status(status)
                .header("retry-after", "0")
                .body(ureq::Body::builder().data("{\"ok\": true}"))
                .unwrap())
        });
        assert_eq!(answer.unwrap(), json!({"ok": true}));
        assert_eq!(tries.get(), 3);

        tries.set(0);
        let refused = with_retries("http://test", || {
            tries.set(tries.get() + 1);
            Err(ureq::Error::ConnectionFailed)
        });
        assert!(refused.is_err());
        assert_eq!(tries.get(), 3, "the first try and two more");
    }
}
