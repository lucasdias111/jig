//! Debug adapters: the Debug Adapter Protocol, framed like LSP, over an
//! adapter's stdin and stdout (`lldb-dap`) or a TCP connection to an adapter
//! server (`js-debug`).
//!
//! Everything an adapter says, responses included, arrives as a
//! [`Message`] on one channel, in order, tagged with the id of the
//! connection it came on: one debug session can have several, as js-debug
//! runs the program in a child session of its own. The debug session is a
//! state machine over those messages, so nothing waits on a reply.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use futures::channel::mpsc::UnboundedSender;
use serde_json::{Value, json};

/// Where a client's messages go: `(connection id, message)`.
pub type Inbox = UnboundedSender<(usize, Message)>;

/// How long a starting adapter server gets to start listening.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Response {
        /// The `seq` [`Client::request`] returned.
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
    /// The adapter asking for a child session, which [`Client::respond`]
    /// answers. Other requests from the adapter are declined for it.
    StartDebugging {
        seq: i64,
        configuration: Value,
    },
    /// The adapter exited or hung up.
    Closed,
}

pub struct Client {
    writer: mpsc::Sender<Vec<u8>>,
    next_seq: AtomicI64,
    child: Mutex<Option<Child>>,
}

impl Client {
    /// Start the adapter at `program`, talking over its stdin and stdout.
    pub fn start(id: usize, program: &Path, inbox: Inbox) -> Result<Self> {
        let mut child = Command::new(program)
            .env("PATH", crate::lsp::search_path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Couldn't start {}", program.display()))?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        let client = Self::connect(id, stdout, stdin, inbox);
        *client.child.lock().unwrap() = Some(child);
        Ok(client)
    }

    /// Connect to an adapter server on `port`, retrying while it starts.
    /// Requests sent meanwhile wait, so the caller can carry on at once.
    pub fn connect_tcp(id: usize, port: u16, inbox: Inbox) -> Self {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let replies = tx.clone();
        std::thread::spawn(move || {
            let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
            let started = Instant::now();
            let stream = loop {
                match TcpStream::connect(address) {
                    Ok(stream) => break stream,
                    Err(_) if started.elapsed() < CONNECT_TIMEOUT => {
                        std::thread::sleep(Duration::from_millis(50));
                    }
                    Err(_) => {
                        let _ = inbox.unbounded_send((id, Message::Closed));
                        return;
                    }
                }
            };
            let Ok(reader) = stream.try_clone() else {
                let _ = inbox.unbounded_send((id, Message::Closed));
                return;
            };
            spawn_writer(stream, rx);
            read_messages(id, reader, replies, inbox);
        });
        Self::new(tx)
    }

    /// A client over streams already connected to an adapter.
    pub fn connect(
        id: usize,
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
        inbox: Inbox,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        spawn_writer(writer, rx);
        let replies = tx.clone();
        std::thread::spawn(move || read_messages(id, reader, replies, inbox));
        Self::new(tx)
    }

    fn new(writer: mpsc::Sender<Vec<u8>>) -> Self {
        Self {
            writer,
            next_seq: AtomicI64::new(1),
            child: Mutex::new(None),
        }
    }

    /// Send a request; its response comes later with this `seq`.
    pub fn request(&self, command: &str, arguments: Value) -> i64 {
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let request = json!({
            "seq": seq,
            "type": "request",
            "command": command,
            "arguments": arguments,
        });
        let _ = self.writer.send(frame(&request));
        seq
    }

    /// Answer the adapter's request `seq`.
    pub fn respond(&self, seq: i64, command: &str, success: bool) {
        let reply = json!({
            "seq": 0,
            "type": "response",
            "request_seq": seq,
            "command": command,
            "success": success,
        });
        let _ = self.writer.send(frame(&reply));
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        if let Some(child) = self.child.lock().unwrap().take() {
            kill(child);
        }
    }
}

/// An adapter that serves DAP on a TCP port, such as js-debug's
/// `dapDebugServer.js`. Dropping it stops it.
pub struct Server {
    child: Option<Child>,
    pub port: u16,
}

impl Server {
    /// Run `program args… <port> 127.0.0.1` on a free port.
    pub fn start(program: &Path, args: &[&Path]) -> Result<Self> {
        let port = free_port()?;
        let child = Command::new(program)
            .args(args)
            .arg(port.to_string())
            .arg("127.0.0.1")
            .env("PATH", crate::lsp::search_path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Couldn't start {}", program.display()))?;
        Ok(Self {
            child: Some(child),
            port,
        })
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Some(child) = self.child.take() {
            kill(child);
        }
    }
}

fn kill(mut child: Child) {
    let _ = child.kill();
    // Reap it off the main thread.
    std::thread::spawn(move || child.wait());
}

/// A port nothing listens on just now.
fn free_port() -> Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).context("No free port")?;
    Ok(listener.local_addr()?.port())
}

fn spawn_writer(mut writer: impl Write + Send + 'static, queue: mpsc::Receiver<Vec<u8>>) {
    std::thread::spawn(move || {
        for bytes in queue {
            if writer
                .write_all(&bytes)
                .and_then(|()| writer.flush())
                .is_err()
            {
                break;
            }
        }
    });
}

/// Pass on what the adapter says until it hangs up.
fn read_messages(id: usize, reader: impl Read, replies: mpsc::Sender<Vec<u8>>, inbox: Inbox) {
    let mut reader = BufReader::new(reader);
    while let Some(message) = read_message(&mut reader) {
        let parsed = match message["type"].as_str() {
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
                let reply = json!({
                    "seq": 0,
                    "type": "response",
                    "request_seq": message["seq"],
                    "command": message["command"],
                    "success": false,
                    "message": "Jig doesn't support this request.",
                });
                let _ = replies.send(frame(&reply));
                continue;
            }
            _ => continue,
        };
        if inbox.unbounded_send((id, parsed)).is_err() {
            return;
        }
    }
    let _ = inbox.unbounded_send((id, Message::Closed));
}

pub(crate) fn frame(message: &Value) -> Vec<u8> {
    let body = message.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

/// The next message, or `None` once the stream ends or breaks.
pub(crate) fn read_message(reader: &mut impl BufRead) -> Option<Value> {
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

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Write};
    use std::os::unix::net::UnixStream;

    use futures::StreamExt as _;
    use serde_json::json;

    use super::*;

    #[test]
    fn requests_go_out_framed_and_replies_come_back_in_order() {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let client = Client::connect(7, ours.try_clone().unwrap(), ours, tx);

        let seq = client.request("initialize", json!({"adapterID": "lldb-dap"}));
        let mut adapter = BufReader::new(theirs.try_clone().unwrap());
        let request = read_message(&mut adapter).unwrap();
        assert_eq!(request["command"], "initialize");
        assert_eq!(request["seq"], seq);

        let mut out = theirs;
        for message in [
            json!({"seq": 1, "type": "event", "event": "initialized"}),
            json!({"seq": 2, "type": "response", "request_seq": seq, "success": true,
                   "command": "initialize", "body": {"supportsStepBack": false}}),
            json!({"seq": 3, "type": "request", "command": "runInTerminal", "arguments": {}}),
        ] {
            out.write_all(&frame(&message)).unwrap();
        }
        // The reverse request is declined.
        let reply = read_message(&mut adapter).unwrap();
        assert_eq!(reply["request_seq"], 3);
        assert_eq!(reply["success"], false);
        drop(out);
        drop(adapter);
        drop(client);

        let messages: Vec<Message> = futures::executor::block_on(rx.collect::<Vec<_>>())
            .into_iter()
            .map(|(id, message)| {
                assert_eq!(id, 7);
                message
            })
            .collect();
        assert_eq!(
            messages,
            [
                Message::Event {
                    event: "initialized".into(),
                    body: Value::Null
                },
                Message::Response {
                    request_seq: seq,
                    success: true,
                    body: json!({"supportsStepBack": false}),
                    message: None,
                },
                Message::Closed,
            ]
        );
    }

    /// Talks to the real `lldb-dap` about a tiny Rust program: the order of
    /// messages the debug session relies on. Needs Xcode's tools and rustc.
    #[test]
    #[ignore]
    fn lldb_dap_live() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("main.rs");
        std::fs::write(
            &source,
            "fn main() {\n    let numbers = vec![1, 2, 3];\n    let total: i32 = numbers.iter().sum();\n    println!(\"total {total}\");\n}\n",
        )
        .unwrap();
        let program = dir.path().join("main");
        let built = std::process::Command::new("rustc")
            .args(["-g", "-o"])
            .arg(&program)
            .arg(&source)
            .status()
            .unwrap();
        assert!(built.success());

        let adapter = crate::debuggers::lldb_dap().expect("lldb-dap");
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let client = Client::start(0, &adapter, tx).unwrap();
        let mut rx = futures::executor::block_on_stream(rx).map(|(_, message)| message);
        let mut log = Vec::new();
        let mut next = |log: &mut Vec<String>| {
            let message = rx.next().expect("adapter hung up");
            log.push(format!("{message:?}").chars().take(160).collect());
            message
        };
        let initialize = client.request("initialize", json!({"adapterID": "lldb-dap", "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path"}));
        loop {
            if let Message::Response {
                request_seq,
                success,
                ..
            } = next(&mut log)
                && request_seq == initialize
            {
                assert!(success);
                break;
            }
        }
        let config = crate::run_configs::RunConfig {
            name: "t".into(),
            command: String::new(),
            cwd: dir.path().to_path_buf(),
            env: Vec::new(),
            source: crate::run_configs::Source::File,
            debug: None,
        };
        let launch = client.request(
            "launch",
            crate::debug_target::DebugTarget::Program {
                build: None,
                program: program.clone(),
                args: Vec::new(),
            }
            .launch_arguments(&config, &program),
        );
        let mut stopped_thread = None;
        while stopped_thread.is_none() {
            match next(&mut log) {
                Message::Event { event, .. } if event == "initialized" => {
                    client.request(
                        "setBreakpoints",
                        json!({"source": {"path": source}, "breakpoints": [{"line": 4}]}),
                    );
                    client.request("configurationDone", json!({}));
                }
                Message::Event { event, body } if event == "stopped" => {
                    stopped_thread = body["threadId"].as_i64();
                }
                Message::Response {
                    request_seq,
                    success,
                    message,
                    ..
                } if request_seq == launch => {
                    assert!(success, "launch failed: {message:?} {log:#?}");
                }
                Message::Closed => panic!("closed early: {log:#?}"),
                _ => {}
            }
        }
        let trace = client.request(
            "stackTrace",
            json!({"threadId": stopped_thread.unwrap(), "levels": 20}),
        );
        let frame = loop {
            if let Message::Response {
                request_seq, body, ..
            } = next(&mut log)
                && request_seq == trace
            {
                let top = &body["stackFrames"][0];
                assert_eq!(top["line"], 4, "{body}");
                assert!(top["source"]["path"].as_str().unwrap().ends_with("main.rs"));
                break top["id"].as_i64().unwrap();
            }
        };
        let scopes = client.request("scopes", json!({"frameId": frame}));
        let locals = loop {
            if let Message::Response {
                request_seq, body, ..
            } = next(&mut log)
                && request_seq == scopes
            {
                break body["scopes"][0]["variablesReference"].as_i64().unwrap();
            }
        };
        let variables = client.request("variables", json!({"variablesReference": locals}));
        loop {
            if let Message::Response {
                request_seq, body, ..
            } = next(&mut log)
                && request_seq == variables
            {
                let names: Vec<&str> = body["variables"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|v| v["name"].as_str())
                    .collect();
                assert!(
                    names.contains(&"total") && names.contains(&"numbers"),
                    "{body}"
                );
                eprintln!("{body:#}");
                break;
            }
        }
        client.request("disconnect", json!({"terminateDebuggee": true}));
        eprintln!("{log:#?}");
    }
}
