//! A fake server, for this crate's tests and jig-app's.

use std::io::{BufReader, Write};
use std::path::PathBuf;
use std::sync::{Arc, mpsc};

use serde_json::{Value, json};

use super::Client;
use crate::domain::protocol::read_message;

/// A fake server on the other end of a pair of pipes. `answer` gets each
/// request's method and params and returns its result.
pub fn fake_server(
    root: PathBuf,
    answer: impl Fn(&str, &Value) -> Value + Send + 'static,
) -> (Arc<Client>, mpsc::Receiver<Value>) {
    let (client_reader, mut server_writer) = std::io::pipe().unwrap();
    let (server_reader, client_writer) = std::io::pipe().unwrap();
    let (seen_tx, seen) = mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(server_reader);
        while let Some(message) = read_message(&mut reader) {
            let method = message["method"].as_str().unwrap_or_default().to_string();
            if let Some(id) = message.get("id") {
                let result = if method == "initialize" {
                    json!({"capabilities": {
                        "positionEncoding": "utf-8",
                        "completionProvider": {"triggerCharacters": ["."]},
                        "documentFormattingProvider": true,
                        "documentRangeFormattingProvider": {},
                    }})
                } else {
                    answer(&method, &message["params"])
                };
                let reply = json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();
                let framed = format!("Content-Length: {}\r\n\r\n{reply}", reply.len());
                if server_writer.write_all(framed.as_bytes()).is_err() {
                    break;
                }
            }
            let _ = seen_tx.send(message);
        }
    });
    (Client::connect(root, client_reader, client_writer), seen)
}
