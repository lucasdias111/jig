//! The Debug Adapter Protocol's messages, framed like LSP, and what each
//! one an adapter sends means for the session.

use std::io::BufRead;

use serde_json::{Value, json};

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Response {
        /// The `seq` `Client::request` returned.
        request_seq: i64,
        success: bool,
        body: Value,
        /// Why it failed, in the adapter's words.
        message: Option<String>,
    },
    Event {
        event: String,
        body: Value,
    },
    /// The adapter asking for a child session, which `Client::respond`
    /// answers. Other requests from the adapter are declined for it.
    StartDebugging {
        seq: i64,
        configuration: Value,
    },
    /// The adapter couldn't be reached, and why.
    Failed(String),
    /// The adapter exited or hung up.
    Closed,
}

/// What one message from an adapter comes to.
pub enum Incoming {
    /// Something for the session.
    Message(Message),
    /// A request Jig declines, with the reply that says so.
    Decline(Value),
    /// Nothing Jig uses.
    Ignored,
}

/// What `message`, as the adapter sent it, comes to.
pub fn parse(message: &Value) -> Incoming {
    Incoming::Message(match message["type"].as_str() {
        Some("response") => Message::Response {
            request_seq: message["request_seq"].as_i64().unwrap_or(-1),
            success: message["success"].as_bool().unwrap_or(false),
            body: message.get("body").cloned().unwrap_or(Value::Null),
            message: message["message"].as_str().map(str::to_string),
        },
        Some("event") => Message::Event {
            event: message["event"].as_str().unwrap_or_default().to_string(),
            body: message.get("body").cloned().unwrap_or(Value::Null),
        },
        Some("request") if message["command"] == "startDebugging" => Message::StartDebugging {
            seq: message["seq"].as_i64().unwrap_or(-1),
            configuration: message["arguments"]["configuration"].clone(),
        },
        // Anything else the adapter asks, such as to run the program in
        // a terminal: decline, so it doesn't wait.
        Some("request") => {
            return Incoming::Decline(json!({
                "seq": 0,
                "type": "response",
                "request_seq": message["seq"],
                "command": message["command"],
                "success": false,
                "message": "Jig doesn't support this request.",
            }));
        }
        _ => return Incoming::Ignored,
    })
}

pub fn frame(message: &Value) -> Vec<u8> {
    let body = message.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

/// The next message, or `None` once the stream ends or breaks.
pub fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).ok()? == 0 {
            return None;
        }
        let header = header.trim_end();
        if header.is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    reader.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}
