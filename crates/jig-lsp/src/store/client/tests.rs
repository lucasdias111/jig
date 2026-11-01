use std::io::{BufReader, Write};
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{Value, json};

use super::testing::fake_server;
use super::*;
use crate::domain::protocol::read_message;

#[test]
fn initializes_then_sends_what_was_queued() {
    let (client, seen) = fake_server(
        PathBuf::from("/project"),
        |method, _| json!({"echo": method}),
    );
    client.open("file:///project/a.rs", "rust", "fn a() {}");
    client.change("file:///project/a.rs", "fn b() {}");
    // The same text again isn't sent.
    client.change("file:///project/a.rs", "fn b() {}");
    let response =
        futures::executor::block_on(client.request("textDocument/definition", json!({})));
    assert_eq!(
        response.unwrap().unwrap(),
        json!({"echo": "textDocument/definition"})
    );
    assert_eq!(client.encoding(), Encoding::Utf8);
    assert_eq!(client.completion_triggers(), ["."]);
    assert_eq!(
        client.formatting(),
        Formatting {
            document: true,
            range: true
        }
    );

    let methods: Vec<String> = seen
        .iter()
        .take(5)
        .map(|m| m["method"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        methods,
        [
            "initialize",
            "initialized",
            "textDocument/didOpen",
            "textDocument/didChange",
            "textDocument/definition"
        ]
    );
}

#[test]
fn documents_open_once_and_close_with_the_last_editor() {
    let (client, seen) = fake_server(PathBuf::from("/project"), |_, _| Value::Null);
    let uri = "file:///project/a.rs";
    client.open(uri, "rust", "");
    client.open(uri, "rust", "");
    client.close(uri);
    client.close(uri);
    futures::executor::block_on(client.request("ping", json!({})))
        .unwrap()
        .unwrap();
    let methods: Vec<String> = seen
        .iter()
        .take(5)
        .map(|m| m["method"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(
        methods,
        [
            "initialize",
            "initialized",
            "textDocument/didOpen",
            "textDocument/didClose",
            "ping"
        ]
    );
}

#[cfg(test)]
/// Answers `test/publish` by first sending its params as a
/// `publishDiagnostics`, so they've been read once the answer is.
fn publishing_server() -> Arc<Client> {
    let (client_reader, mut server_writer) = std::io::pipe().unwrap();
    let (server_reader, client_writer) = std::io::pipe().unwrap();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(server_reader);
        let mut write = |message: Value| {
            let body = message.to_string();
            let framed = format!("Content-Length: {}\r\n\r\n{body}", body.len());
            server_writer.write_all(framed.as_bytes())
        };
        while let Some(message) = read_message(&mut reader) {
            let Some(id) = message.get("id") else {
                continue;
            };
            if message["method"] == "test/publish" {
                let notification = json!({
                    "jsonrpc": "2.0",
                    "method": "textDocument/publishDiagnostics",
                    "params": message["params"],
                });
                if write(notification).is_err() {
                    break;
                }
            }
            let result = if message["method"] == "initialize" {
                json!({"capabilities": {}})
            } else {
                Value::Null
            };
            if write(json!({"jsonrpc": "2.0", "id": id, "result": result})).is_err() {
                break;
            }
        }
    });
    Client::connect(PathBuf::from("/project"), client_reader, client_writer)
}

#[test]
fn reports_diagnostics_and_drops_stale_ones() {
    let client = publishing_server();
    let (sink, mut reports) = channel::unbounded();
    client.on_diagnostics(sink);
    let uri = "file:///project/a.rs";
    let problem = json!([{
        "range": {"start": {"line": 0, "character": 3}, "end": {"line": 0, "character": 4}},
        "severity": 1,
        "message": "expected `;`",
    }]);
    let publish = |params: Value| {
        futures::executor::block_on(client.request("test/publish", params))
            .unwrap()
            .unwrap();
    };
    client.open(uri, "rust", "fn a");
    publish(json!({"uri": uri, "version": 0, "diagnostics": problem}));
    let published = reports.try_recv().unwrap();
    assert_eq!(published.key, crate::store::diagnostics::uri_key(uri));
    assert_eq!(published.items[0].message, "expected `;`");
    assert_eq!(published.encoding, Encoding::Utf16);

    // About text from before the latest change: dropped.
    client.change(uri, "fn a()");
    publish(json!({"uri": uri, "version": 0, "diagnostics": problem}));
    assert!(reports.try_recv().is_err(), "nothing waiting");
    // Without a version, or for the current one: taken. Empty clears.
    publish(json!({"uri": uri, "diagnostics": []}));
    assert!(reports.try_recv().unwrap().items.is_empty());
    publish(json!({"uri": uri, "version": 1, "diagnostics": problem}));
    assert_eq!(reports.try_recv().unwrap().version, Some(1));
}

#[test]
fn a_server_that_cant_start_fails_requests() {
    let client = Client::start("nothing", PathBuf::from("/"), Value::Null, || {
        Some(Command::new("/nonexistent/jig-test-server"))
    });
    let response = futures::executor::block_on(client.request("x", json!({})));
    assert!(matches!(response, Ok(Err(_))));
    assert!(client.is_dead());
    let failure = client.failure().unwrap();
    assert!(failure.starts_with("nothing couldn't start:"), "{failure}");
}

#[cfg(unix)]
#[test]
fn a_server_that_exits_as_it_starts_says_why() {
    let client = Client::start("rust-analyzer", PathBuf::from("/"), Value::Null, || {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "echo \"error: Unknown binary 'rust-analyzer'\" >&2; exit 1",
        ]);
        Some(command)
    });
    let (sink, mut told) = channel::unbounded();
    client.on_failure(sink);
    let started = std::time::Instant::now();
    while told.try_recv().is_err() {
        assert!(started.elapsed().as_secs() < 5, "never told");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(
        client.failure().as_deref(),
        Some("rust-analyzer didn't start: Unknown binary 'rust-analyzer'")
    );
}
