//! One running language server: JSON-RPC over its stdin and stdout.
//!
//! Messages sent before the server has answered `initialize` wait in a
//! queue, so callers never have to. A reader thread routes responses to
//! whoever asked and answers the few requests servers send back.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use futures::channel::{mpsc as channel, oneshot};
use serde_json::{Value, json};

use super::Encoding;
use super::diagnostics::Published;

/// What a request answers with: the result, or the error in words.
pub type Response = Result<Value, String>;

pub struct Client {
    state: Arc<Mutex<State>>,
    next_id: AtomicI64,
}

struct State {
    phase: Phase,
    /// How positions count columns, once the server has said.
    encoding: Encoding,
    pending: HashMap<i64, oneshot::Sender<Response>>,
    /// Open documents, by URI.
    documents: HashMap<String, Document>,
    /// What typed after a name asks for completions, such as `.`.
    completion_triggers: Vec<String>,
    /// Sent in `initialize`, such as the plugins jdtls loads.
    init_options: Value,
    /// The server said it has loaded the project, as jdtls does.
    service_ready: bool,
    /// What it can format: whole documents, and selected ranges.
    formatting: Formatting,
    /// Where the problems it reports go.
    diagnostics: Option<channel::UnboundedSender<Published>>,
}

struct Document {
    /// How many editors have it open.
    editors: usize,
    version: i32,
    /// A hash of the text last sent, so the same text isn't sent twice.
    sent: u64,
}

/// Which formatting requests a server answers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Formatting {
    pub document: bool,
    pub range: bool,
}

impl Formatting {
    /// From `initialize`'s capabilities, where each is `true` or options.
    fn from_capabilities(capabilities: &Value) -> Self {
        let offered = |key: &str| match &capabilities[key] {
            Value::Bool(on) => *on,
            Value::Object(_) => true,
            _ => false,
        };
        Self {
            document: offered("documentFormattingProvider"),
            range: offered("documentRangeFormattingProvider"),
        }
    }
}

enum Phase {
    /// Not initialized yet; what to send once it is.
    Starting(Vec<Vec<u8>>),
    Ready(mpsc::Sender<Vec<u8>>),
    /// Exited, or never started. Requests fail at once.
    Dead,
}

const INITIALIZE_ID: i64 = 0;

impl Client {
    /// Start a server in `root` with the command `command` makes, on a
    /// thread, so finding the program doesn't hold up the caller. `None`
    /// means it isn't installed.
    pub fn start(
        name: &'static str,
        root: PathBuf,
        init_options: Value,
        command: impl FnOnce() -> Option<Command> + Send + 'static,
    ) -> Arc<Self> {
        let client = Arc::new(Self::new(init_options));
        let state = client.state.clone();
        std::thread::spawn(move || {
            let Some(mut command) = command() else {
                die(&state);
                return;
            };
            command
                .current_dir(&root)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            match command.spawn() {
                Ok(mut child) => {
                    let stdin = child.stdin.take().expect("piped");
                    let stdout = child.stdout.take().expect("piped");
                    connect(&state, &root, stdout, stdin);
                    let _ = child.wait();
                }
                Err(error) => {
                    eprintln!("jig: couldn't start {name}: {error}");
                    die(&state);
                }
            }
        });
        client
    }

    /// A client over streams already connected to a server, for tests.
    #[cfg(test)]
    pub fn connect(
        root: PathBuf,
        reader: impl Read + Send + 'static,
        writer: impl Write + Send + 'static,
    ) -> Arc<Self> {
        let client = Arc::new(Self::new(Value::Null));
        let state = client.state.clone();
        std::thread::spawn(move || connect(&state, &root, reader, writer));
        client
    }

    fn new(init_options: Value) -> Self {
        Self {
            state: Arc::new(Mutex::new(State {
                phase: Phase::Starting(Vec::new()),
                encoding: Encoding::Utf16,
                pending: HashMap::new(),
                documents: HashMap::new(),
                completion_triggers: Vec::new(),
                init_options,
                service_ready: false,
                formatting: Formatting::default(),
                diagnostics: None,
            })),
            // 0 is `initialize`.
            next_id: AtomicI64::new(1),
        }
    }

    /// What it was started with, in `initialize`.
    pub fn init_options(&self) -> Value {
        self.state.lock().unwrap().init_options.clone()
    }

    /// Whether the server has said it loaded the project. Only some say so.
    pub fn is_service_ready(&self) -> bool {
        self.state.lock().unwrap().service_ready
    }

    /// Send the problems the server reports to `sink`.
    pub fn on_diagnostics(&self, sink: channel::UnboundedSender<Published>) {
        self.state.lock().unwrap().diagnostics = Some(sink);
    }

    pub fn is_dead(&self) -> bool {
        matches!(self.state.lock().unwrap().phase, Phase::Dead)
    }

    /// How the server counts columns. UTF-16 until it says otherwise.
    pub fn encoding(&self) -> Encoding {
        self.state.lock().unwrap().encoding
    }

    /// What the server formats. Nothing until it has started.
    pub fn formatting(&self) -> Formatting {
        self.state.lock().unwrap().formatting
    }

    /// What typed after a name asks for completions, such as `.`.
    pub fn completion_triggers(&self) -> Vec<String> {
        self.state.lock().unwrap().completion_triggers.clone()
    }

    pub fn request(&self, method: &str, params: Value) -> oneshot::Receiver<Response> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        let mut state = self.state.lock().unwrap();
        if matches!(state.phase, Phase::Dead) {
            let _ = tx.send(Err("the language server isn't running".into()));
            return rx;
        }
        state.pending.insert(id, tx);
        send(
            &mut state,
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
        );
        rx
    }

    pub fn notify(&self, method: &str, params: Value) {
        let mut state = self.state.lock().unwrap();
        send(
            &mut state,
            json!({"jsonrpc": "2.0", "method": method, "params": params}),
        );
    }

    /// An editor opened `uri`. Only the first to open it tells the server.
    pub fn open(&self, uri: &str, language_id: &str, text: &str) {
        let first = {
            let mut state = self.state.lock().unwrap();
            let document = state.documents.entry(uri.to_string()).or_insert(Document {
                editors: 0,
                version: 0,
                sent: hash(text),
            });
            document.editors += 1;
            document.editors == 1
        };
        if first {
            self.notify(
                "textDocument/didOpen",
                json!({"textDocument": {
                    "uri": uri, "languageId": language_id, "version": 0, "text": text,
                }}),
            );
        }
    }

    /// `uri` now reads `text`. Nothing is sent if the server already has it.
    pub fn change(&self, uri: &str, text: &str) {
        let sent = hash(text);
        let version = {
            let mut state = self.state.lock().unwrap();
            let Some(document) = state.documents.get_mut(uri) else {
                return;
            };
            if document.sent == sent {
                return;
            }
            document.sent = sent;
            document.version += 1;
            document.version
        };
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": {"uri": uri, "version": version},
                "contentChanges": [{"text": text}],
            }),
        );
    }

    pub fn save(&self, uri: &str) {
        self.notify(
            "textDocument/didSave",
            json!({"textDocument": {"uri": uri}}),
        );
    }

    /// An editor closed `uri`. The last to close it tells the server.
    pub fn close(&self, uri: &str) {
        let last = {
            let mut state = self.state.lock().unwrap();
            match state.documents.get_mut(uri) {
                Some(document) if document.editors > 1 => {
                    document.editors -= 1;
                    false
                }
                Some(_) => {
                    state.documents.remove(uri);
                    true
                }
                None => false,
            }
        };
        if last {
            self.notify(
                "textDocument/didClose",
                json!({"textDocument": {"uri": uri}}),
            );
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Polite, then final; the server exits when its stdin closes anyway.
        let mut state = self.state.lock().unwrap();
        send(
            &mut state,
            json!({"jsonrpc": "2.0", "id": i64::MAX, "method": "shutdown"}),
        );
        send(&mut state, json!({"jsonrpc": "2.0", "method": "exit"}));
        state.phase = Phase::Dead;
    }
}

fn send(state: &mut State, message: Value) {
    let body = message.to_string();
    let bytes = format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes();
    match &mut state.phase {
        Phase::Starting(queue) => queue.push(bytes),
        Phase::Ready(writer) => {
            let _ = writer.send(bytes);
        }
        Phase::Dead => {}
    }
}

fn die(state: &Mutex<State>) {
    let mut state = state.lock().unwrap();
    state.phase = Phase::Dead;
    for (_, pending) in state.pending.drain() {
        let _ = pending.send(Err("the language server exited".into()));
    }
}

/// Talk to a server over `reader` and `writer` until it hangs up.
fn connect(
    state: &Arc<Mutex<State>>,
    root: &Path,
    reader: impl Read,
    mut writer: impl Write + Send + 'static,
) {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        for bytes in rx {
            if writer
                .write_all(&bytes)
                .and_then(|_| writer.flush())
                .is_err()
            {
                break;
            }
        }
    });

    let init_options = state.lock().unwrap().init_options.clone();
    let root_uri = super::file_uri(root);
    let name = root
        .file_name()
        .map_or_else(|| "root".into(), |name| name.to_string_lossy().into_owned());
    let initialize = json!({
        "jsonrpc": "2.0",
        "id": INITIALIZE_ID,
        "method": "initialize",
        "params": {
            "processId": std::process::id(),
            "clientInfo": {"name": "Jig"},
            "initializationOptions": init_options,
            "rootPath": root,
            "rootUri": root_uri,
            "workspaceFolders": [{"uri": root_uri, "name": name}],
            "capabilities": {
                "general": {"positionEncodings": ["utf-8", "utf-16"]},
                "textDocument": {
                    "synchronization": {"didSave": true, "dynamicRegistration": false},
                    "definition": {"linkSupport": true},
                    "completion": {
                        "completionItem": {
                            "snippetSupport": true,
                            "deprecatedSupport": true,
                            "documentationFormat": ["markdown", "plaintext"],
                        },
                        "contextSupport": true,
                    },
                    "references": {},
                    "formatting": {},
                    "rangeFormatting": {},
                    "hover": {"contentFormat": ["markdown", "plaintext"]},
                    "signatureHelp": {
                        "signatureInformation": {
                            "documentationFormat": ["markdown", "plaintext"],
                            "parameterInformation": {"labelOffsetSupport": true},
                            "activeParameterSupport": true,
                        },
                        "contextSupport": true,
                    },
                },
                "workspace": {"workspaceFolders": true, "configuration": true},
                "window": {"workDoneProgress": false},
            },
        },
    });
    let body = initialize.to_string();
    let _ = tx.send(format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes());

    let mut reader = BufReader::new(reader);
    while let Some(message) = read_message(&mut reader) {
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);
        match (id, method) {
            // The server asking us something: answer so it doesn't wait.
            (Some(id), Some(method)) => {
                let result = match method {
                    "workspace/configuration" => {
                        let items = message["params"]["items"].as_array().map_or(0, Vec::len);
                        Value::Array(vec![Value::Null; items])
                    }
                    "workspace/workspaceFolders" => {
                        json!([{"uri": root_uri, "name": name}])
                    }
                    _ => Value::Null,
                };
                let reply = json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();
                let _ =
                    tx.send(format!("Content-Length: {}\r\n\r\n{reply}", reply.len()).into_bytes());
            }
            (Some(id), None) => {
                let response = match message.get("error") {
                    Some(error) => Err(error["message"]
                        .as_str()
                        .unwrap_or("the language server failed")
                        .to_string()),
                    None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                };
                if id.as_i64() == Some(INITIALIZE_ID) {
                    let Ok(result) = response else {
                        break;
                    };
                    let encoding = match result["capabilities"]["positionEncoding"].as_str() {
                        Some("utf-8") => Encoding::Utf8,
                        _ => Encoding::Utf16,
                    };
                    let triggers =
                        result["capabilities"]["completionProvider"]["triggerCharacters"]
                            .as_array()
                            .map(|triggers| {
                                triggers
                                    .iter()
                                    .filter_map(|t| Some(t.as_str()?.to_string()))
                                    .collect()
                            })
                            .unwrap_or_default();
                    let mut state = state.lock().unwrap();
                    state.encoding = encoding;
                    state.completion_triggers = triggers;
                    state.formatting = Formatting::from_capabilities(&result["capabilities"]);
                    let initialized =
                        json!({"jsonrpc": "2.0", "method": "initialized", "params": {}})
                            .to_string();
                    let _ = tx.send(
                        format!("Content-Length: {}\r\n\r\n{initialized}", initialized.len())
                            .into_bytes(),
                    );
                    if let Phase::Starting(queue) =
                        std::mem::replace(&mut state.phase, Phase::Ready(tx.clone()))
                    {
                        for bytes in queue {
                            let _ = tx.send(bytes);
                        }
                    }
                } else if let Some(pending) = id
                    .as_i64()
                    .and_then(|id| state.lock().unwrap().pending.remove(&id))
                {
                    let _ = pending.send(response);
                }
            }
            // jdtls says when it has loaded the project, which its
            // commands need first.
            (None, Some("language/status")) if message["params"]["type"] == "ServiceReady" => {
                state.lock().unwrap().service_ready = true;
            }
            (None, Some("textDocument/publishDiagnostics")) => {
                let state = state.lock().unwrap();
                if let Some(published) =
                    super::diagnostics::parse(&message["params"], state.encoding)
                    && let Some(sink) = &state.diagnostics
                    && !is_stale(&state, &published)
                {
                    let _ = sink.unbounded_send(published);
                }
            }
            // Other notifications: progress, logs. Not used.
            _ => {}
        }
    }
    die(state);
}

/// Whether `published` is about text that has changed since: the server
/// sends again for the newer text, and these would land in the wrong
/// places. Reports without a version are always taken.
fn is_stale(state: &State, published: &Published) -> bool {
    let Some(version) = published.version else {
        return false;
    };
    state
        .documents
        .iter()
        .find(|(uri, _)| super::diagnostics::uri_key(uri) == published.key)
        .is_some_and(|(_, document)| document.version > version)
}

fn hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// One `Content-Length`-framed message, or `None` once the stream ends.
fn read_message(reader: &mut impl BufRead) -> Option<Value> {
    loop {
        let mut length = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).ok()? == 0 {
                return None;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some((key, value)) = line.split_once(':')
                && key.eq_ignore_ascii_case("content-length")
            {
                length = value.trim().parse::<usize>().ok();
            }
        }
        let Some(length) = length else {
            continue;
        };
        let mut body = vec![0; length];
        reader.read_exact(&mut body).ok()?;
        // Skip anything that isn't JSON rather than give up on the server.
        if let Ok(message) = serde_json::from_slice(&body) {
            return Some(message);
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

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
        assert_eq!(published.key, super::super::diagnostics::uri_key(uri));
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
    }
}
