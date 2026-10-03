//! Provider tests against a one-shot local HTTP server, so no network or
//! API keys are needed.

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;

use jig_ai::config::AuthStyle;
use jig_ai::{AnthropicProvider, OpenAiCompatProvider, PromptRequest, Provider};

struct Recorded {
    request_line: String,
    headers: Vec<(String, String)>,
    body: serde_json::Value,
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
                body: serde_json::from_slice(&raw).unwrap(),
            })
            .unwrap();
        }
    });
    (url, rx)
}

fn request() -> PromptRequest {
    PromptRequest {
        instruction: "Rename to b".into(),
        language: "rust".into(),
        file_name: Some("lib.rs".into()),
        text: "fn a() {}\n".into(),
        target: 0..9,
        comment: None,
        project_rules: None,
    }
}

#[test]
fn openai_compat_round_trip() {
    let (url, rx) = serve_once(
        200,
        r#"{"choices":[{"message":{"role":"assistant","content":"{\"replace\":\"fn b() {}\",\"message\":\"Renamed.\"}"}}]}"#,
    );
    let provider =
        OpenAiCompatProvider::new(&format!("{url}/v1/"), Some("k".into()), "glm", 1000, true)
            .with_headers(vec![("x-opencode-session".into(), "jig-1".into())]);
    let reply = jig_ai::run(&provider, &request()).unwrap();
    assert_eq!(reply.replace, "fn b() {}");
    assert_eq!(reply.message, "Renamed.");

    let sent = rx.recv().unwrap();
    assert_eq!(sent.request_line, "POST /v1/chat/completions HTTP/1.1");
    assert_eq!(sent.header("authorization"), Some("Bearer k"));
    assert_eq!(sent.header("x-opencode-session"), Some("jig-1"));
    assert!(sent.header("user-agent").unwrap().starts_with("jig/"));
    assert_eq!(sent.body["model"], "glm");
    assert_eq!(sent.body["response_format"]["type"], "json_object");
    assert_eq!(sent.body["messages"][0]["role"], "system");
    assert!(
        sent.body["messages"][1]["content"]
            .as_str()
            .unwrap()
            .contains("<<<SELECTION>>>fn a() {}<<<END>>>")
    );
}

#[test]
fn openai_compat_without_key_or_json_mode() {
    let (url, rx) = serve_once(200, r#"{"choices":[{"message":{"content":"hi"}}]}"#);
    let provider = OpenAiCompatProvider::new(&url, None, "m", 1000, false);
    assert_eq!(provider.complete("s", "u").unwrap(), "hi");
    let sent = rx.recv().unwrap();
    assert_eq!(sent.header("authorization"), None);
    assert!(sent.body.get("response_format").is_none());
}

#[test]
fn anthropic_round_trip_with_api_key() {
    let (url, rx) = serve_once(
        200,
        r#"{"content":[{"type":"text","text":"{\"replace\":\"fn b() {}\","},{"type":"text","text":"\"message\":\"Renamed.\"}"}]}"#,
    );
    let provider = AnthropicProvider::new(&url, "k".into(), "claude", 1000, AuthStyle::ApiKey);
    let reply = jig_ai::run(&provider, &request()).unwrap();
    assert_eq!(reply.replace, "fn b() {}");

    let sent = rx.recv().unwrap();
    assert_eq!(sent.request_line, "POST /messages HTTP/1.1");
    assert_eq!(sent.header("x-api-key"), Some("k"));
    assert_eq!(sent.header("anthropic-version"), Some("2023-06-01"));
    assert!(sent.body["system"].as_str().unwrap().contains("Jig"));
    assert_eq!(sent.body["messages"][0]["role"], "user");
}

#[test]
fn anthropic_bearer_auth() {
    let (url, rx) = serve_once(200, r#"{"content":[{"type":"text","text":"x"}]}"#);
    let provider = AnthropicProvider::new(&url, "k".into(), "qwen", 1000, AuthStyle::Bearer);
    provider.complete("s", "u").unwrap();
    let sent = rx.recv().unwrap();
    assert_eq!(sent.header("authorization"), Some("Bearer k"));
    assert_eq!(sent.header("x-api-key"), None);
}

#[test]
fn http_error_carries_provider_message() {
    let (url, _rx) = serve_once(401, r#"{"error":{"message":"invalid api key"}}"#);
    let provider = OpenAiCompatProvider::new(&url, Some("bad".into()), "m", 1000, true);
    let error = provider.complete("s", "u").unwrap_err().to_string();
    assert!(error.contains("401"), "{error}");
    assert!(error.contains("invalid api key"), "{error}");
}

#[test]
fn unreachable_server_is_an_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let provider = OpenAiCompatProvider::new(&url, None, "m", 1000, true);
    assert!(
        provider
            .complete("s", "u")
            .unwrap_err()
            .to_string()
            .contains("Couldn't reach")
    );
}

/// Answers every tool call with a fixed text and remembers the calls.
struct FakeTools(std::sync::Mutex<Vec<(String, serde_json::Value)>>);

impl jig_ai::ToolHost for FakeTools {
    fn specs(&self) -> Vec<jig_ai::ToolSpec> {
        vec![jig_ai::ToolSpec {
            name: "read_file",
            description: "Read a file.",
            input_schema: serde_json::json!({ "type": "object", "properties": { "path": { "type": "string" } } }),
        }]
    }

    fn call(&self, name: &str, input: &serde_json::Value) -> String {
        self.0
            .lock()
            .unwrap()
            .push((name.to_string(), input.clone()));
        "pub struct User;".into()
    }

    fn describe(&self, _: &str, input: &serde_json::Value) -> String {
        format!("Reading {}", input["path"].as_str().unwrap_or("?"))
    }
}

fn fake_tools() -> FakeTools {
    FakeTools(Default::default())
}

#[test]
fn anthropic_tool_loop() {
    let (url, rx) = serve(vec![
        (
            200,
            r#"{"content":[{"type":"text","text":"Let me look."},{"type":"tool_use","id":"t1","name":"read_file","input":{"path":"src/user.rs"}}],"stop_reason":"tool_use"}"#,
        ),
        (
            200,
            r#"{"content":[{"type":"text","text":"{\"replace\":\"fn b() {}\",\"message\":\"Done.\"}"}],"stop_reason":"end_turn"}"#,
        ),
    ]);
    let provider = AnthropicProvider::new(&url, "k".into(), "qwen", 1000, AuthStyle::ApiKey);
    let tools = fake_tools();
    let steps = std::sync::Mutex::new(Vec::new());
    let reply = jig_ai::run_exploring(&provider, &request(), &tools, &|step| {
        steps.lock().unwrap().push(step)
    })
    .unwrap();
    assert_eq!(reply.replace, "fn b() {}");
    assert_eq!(*steps.lock().unwrap(), ["Reading src/user.rs"]);
    assert_eq!(tools.0.lock().unwrap()[0].1["path"], "src/user.rs");

    let first = rx.recv().unwrap();
    assert_eq!(first.body["tools"][0]["name"], "read_file");
    assert!(first.body["tools"][0]["input_schema"].is_object());
    assert!(
        first.body["system"]
            .as_str()
            .unwrap()
            .contains("read-only tools")
    );
    assert!(first.body.get("tool_choice").is_none());

    let second = rx.recv().unwrap();
    let messages = second.body["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(messages[1]["content"][1]["type"], "tool_use");
    assert_eq!(messages[2]["content"][0]["type"], "tool_result");
    assert_eq!(messages[2]["content"][0]["tool_use_id"], "t1");
    assert_eq!(messages[2]["content"][0]["content"], "pub struct User;");
}

#[test]
fn openai_tool_loop() {
    let (url, rx) = serve(vec![
        (
            200,
            r#"{"choices":[{"message":{"role":"assistant","content":null,"tool_calls":[{"id":"c1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a.rs\"}"}}]}}]}"#,
        ),
        (
            200,
            r#"{"choices":[{"message":{"role":"assistant","content":"{\"replace\":\"fn b() {}\",\"message\":\"Done.\"}"}}]}"#,
        ),
    ]);
    let provider = OpenAiCompatProvider::new(&url, Some("k".into()), "glm", 1000, true);
    let tools = fake_tools();
    let reply = jig_ai::run_exploring(&provider, &request(), &tools, &|_| {}).unwrap();
    assert_eq!(reply.replace, "fn b() {}");

    let first = rx.recv().unwrap();
    assert_eq!(first.body["tools"][0]["type"], "function");
    assert_eq!(first.body["tools"][0]["function"]["name"], "read_file");
    assert!(
        first.body.get("response_format").is_none(),
        "no JSON mode alongside tools"
    );

    let second = rx.recv().unwrap();
    let messages = second.body["messages"].as_array().unwrap();
    assert_eq!(messages[2]["tool_calls"][0]["id"], "c1");
    assert_eq!(messages[3]["role"], "tool");
    assert_eq!(messages[3]["tool_call_id"], "c1");
    assert_eq!(messages[3]["content"], "pub struct User;");
}

#[test]
fn tool_budget_is_enforced() {
    let (url, rx) = serve(vec![
        (
            200,
            r#"{"content":[{"type":"tool_use","id":"t1","name":"read_file","input":{"path":"a"}},{"type":"tool_use","id":"t2","name":"read_file","input":{"path":"b"}}],"stop_reason":"tool_use"}"#,
        ),
        (
            200,
            r#"{"content":[{"type":"text","text":"{\"replace\":\"x\"}"}]}"#,
        ),
    ]);
    let provider = AnthropicProvider::new(&url, "k".into(), "qwen", 1000, AuthStyle::ApiKey);
    let tools = fake_tools();
    let raw =
        jig_ai::Provider::complete_with_tools(&provider, "s", "u", &tools, 1, &|_| {}).unwrap();
    assert_eq!(raw, "{\"replace\":\"x\"}");
    assert_eq!(tools.0.lock().unwrap().len(), 1, "only one call ran");

    rx.recv().unwrap();
    let second = rx.recv().unwrap();
    assert_eq!(
        second.body["tool_choice"]["type"], "none",
        "no more tools once the budget is spent"
    );
    let results = &second.body["messages"][2]["content"];
    assert_eq!(
        results[1]["content"].as_str().unwrap().split('.').next(),
        Some("Tool limit reached")
    );
}
