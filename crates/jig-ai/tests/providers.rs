//! Asking providers for their models, against a one-shot local HTTP server, so no network or
//! API keys are needed.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

struct Recorded {
    request_line: String,
    headers: Vec<(String, String)>,
}

impl Recorded {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Serve one request with `status` and `body`; returns the base URL and a
/// receiver for what the client sent.
fn serve_once(status: u16, body: &'static str) -> (String, mpsc::Receiver<Recorded>) {
    serve(vec![(status, body)])
}

/// Serve one request per response, in order, over separate connections.
fn serve(responses: Vec<(u16, &'static str)>) -> (String, mpsc::Receiver<Recorded>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for (status, body) in responses {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            let mut headers = Vec::new();
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let line = line.trim_end();
                if line.is_empty() {
                    break;
                }
                let (name, value) = line.split_once(':').unwrap();
                headers.push((name.trim().to_string(), value.trim().to_string()));
            }
            let length: usize = headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
                .map(|(_, v)| v.parse().unwrap())
                .unwrap_or(0);
            let mut raw = vec![0; length];
            reader.read_exact(&mut raw).unwrap();
            let mut stream = stream;
            write!(
            stream,
            "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
            tx.send(Recorded {
                request_line: request_line.trim_end().to_string(),
                headers,
            })
            .unwrap();
        }
    });
    (url, rx)
}

#[test]
fn models_are_listed_with_the_providers_auth() {
    let (url, rx) = serve_once(200, r#"{"data":[{"id":"claude-b"},{"id":"claude-a"}]}"#);
    let config = jig_ai::Config::parse(&format!(
        "[[provider]]\nname = \"c\"\nkind = \"anthropic\"\nbase_url = \"{url}/v1/\"\nmodel = \"\"\n"
    ))
    .unwrap();
    let models = config.providers[0].list_models(Some("k")).unwrap();
    assert_eq!(models, ["claude-a", "claude-b"]);
    let sent = rx.recv().unwrap();
    assert_eq!(sent.request_line, "GET /v1/models HTTP/1.1");
    assert_eq!(sent.header("x-api-key"), Some("k"));
    assert_eq!(sent.header("anthropic-version"), Some("2023-06-01"));
}

fn provider(url: &str) -> jig_ai::ProviderConfig {
    let config = jig_ai::Config::parse(&format!(
        "[[provider]]\nname = \"o\"\nkind = \"openai\"\nbase_url = \"{url}\"\n"
    ))
    .unwrap();
    config.providers[0].clone()
}

#[test]
fn http_error_carries_provider_message() {
    let (url, _rx) = serve_once(401, r#"{"error":{"message":"invalid api key"}}"#);
    let error = provider(&url)
        .list_models(Some("bad"))
        .unwrap_err()
        .to_string();
    assert!(error.contains("401"), "{error}");
    assert!(error.contains("invalid api key"), "{error}");
}

#[test]
fn unreachable_server_is_an_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let error = provider(&url).list_models(None).unwrap_err().to_string();
    assert!(error.contains("Couldn't reach"), "{error}");
}
