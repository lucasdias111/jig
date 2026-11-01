//! One running language server: JSON-RPC over its stdin and stdout.
//!
//! Messages sent before the server has answered `initialize` wait in a
//! queue, so callers never have to. A reader thread routes responses to
//! whoever asked and answers the few requests servers send back.

use std::collections::{HashMap, VecDeque};
#[cfg(any(test, feature = "testing"))]
use std::io::Write;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, MutexGuard};

use futures::channel::{mpsc as channel, oneshot};
use serde_json::{Value, json};

use crate::domain::diagnostics::Published;
use crate::domain::positions::Encoding;
use crate::domain::protocol::{Formatting, failure, frame, hash};
use connection::connect;

mod connection;
#[cfg(any(test, feature = "testing"))]
pub mod testing;
#[cfg(test)]
mod tests;

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
    /// It answered `initialize`, so exiting later isn't failing to start.
    started: bool,
    /// Why it stopped before it started, in its own words.
    failure: Option<String>,
    /// Told once it fails to start.
    on_failure: Option<channel::UnboundedSender<()>>,
}

struct Document {
    /// How many editors have it open.
    editors: usize,
    version: i32,
    /// A hash of the text last sent, so the same text isn't sent twice.
    sent: u64,
}

enum Phase {
    /// Not initialized yet; what to send once it is.
    Starting(Vec<Vec<u8>>),
    Ready(mpsc::Sender<Vec<u8>>),
    /// Exited, or never started. Requests fail at once.
    Dead,
}

/// How much of what a server writes to stderr is kept, to say why it
/// stopped.
const STDERR_LINES: usize = 5;

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
                .stderr(Stdio::piped());
            match command.spawn() {
                Ok(mut child) => {
                    let stdin = child.stdin.take().expect("piped");
                    let stdout = child.stdout.take().expect("piped");
                    let stderr = keep_last_lines(child.stderr.take().expect("piped"));
                    connect(&state, &root, stdout, stdin);
                    let _ = child.wait();
                    if !lock(&state).started {
                        // It has exited, so its stderr has ended too.
                        let lines = stderr.join().unwrap_or_default();
                        fail(&state, failure(name, &lines));
                    }
                }
                Err(error) => {
                    fail(&state, format!("{name} couldn't start: {error}"));
                    die(&state);
                }
            }
        });
        client
    }

    /// A client over streams already connected to a server, for tests.
    #[cfg(any(test, feature = "testing"))]
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
                started: false,
                failure: None,
                on_failure: None,
            })),
            // 0 is `initialize`.
            next_id: AtomicI64::new(1),
        }
    }

    /// What it was started with, in `initialize`.
    pub fn init_options(&self) -> Value {
        lock(&self.state).init_options.clone()
    }

    /// Whether the server has said it loaded the project. Only some say so.
    pub fn is_service_ready(&self) -> bool {
        lock(&self.state).service_ready
    }

    /// Send the problems the server reports to `sink`.
    pub fn on_diagnostics(&self, sink: channel::UnboundedSender<Published>) {
        lock(&self.state).diagnostics = Some(sink);
    }

    /// Tell `sink` if the server stops before it has started, as when a
    /// rustup proxy has no rust-analyzer for the project's toolchain.
    pub fn on_failure(&self, sink: channel::UnboundedSender<()>) {
        let mut state = lock(&self.state);
        if state.failure.is_some() {
            let _ = sink.unbounded_send(());
        }
        state.on_failure = Some(sink);
    }

    /// Why the server stopped before it started, if it did.
    pub fn failure(&self) -> Option<String> {
        lock(&self.state).failure.clone()
    }

    pub fn is_dead(&self) -> bool {
        matches!(lock(&self.state).phase, Phase::Dead)
    }

    /// How the server counts columns. UTF-16 until it says otherwise.
    pub fn encoding(&self) -> Encoding {
        lock(&self.state).encoding
    }

    /// What the server formats. Nothing until it has started.
    pub fn formatting(&self) -> Formatting {
        lock(&self.state).formatting
    }

    /// What typed after a name asks for completions, such as `.`.
    pub fn completion_triggers(&self) -> Vec<String> {
        lock(&self.state).completion_triggers.clone()
    }

    pub fn request(&self, method: &str, params: Value) -> oneshot::Receiver<Response> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        let mut state = lock(&self.state);
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
        let mut state = lock(&self.state);
        send(
            &mut state,
            json!({"jsonrpc": "2.0", "method": method, "params": params}),
        );
    }

    /// An editor opened `uri`. Only the first to open it tells the server.
    pub fn open(&self, uri: &str, language_id: &str, text: &str) {
        let first = {
            let mut state = lock(&self.state);
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
            let mut state = lock(&self.state);
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
            let mut state = lock(&self.state);
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
        let mut state = lock(&self.state);
        send(
            &mut state,
            json!({"jsonrpc": "2.0", "id": i64::MAX, "method": "shutdown"}),
        );
        send(&mut state, json!({"jsonrpc": "2.0", "method": "exit"}));
        state.phase = Phase::Dead;
    }
}

fn send(state: &mut State, message: Value) {
    let bytes = frame(&message);
    match &mut state.phase {
        Phase::Starting(queue) => queue.push(bytes),
        Phase::Ready(writer) => {
            let _ = writer.send(bytes);
        }
        Phase::Dead => {}
    }
}

/// Read `stderr` to its end on a thread, keeping its last few lines.
fn keep_last_lines(stderr: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<String>> {
    std::thread::spawn(move || {
        let mut lines = VecDeque::with_capacity(STDERR_LINES);
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            if lines.len() == STDERR_LINES {
                lines.pop_front();
            }
            lines.push_back(line);
        }
        lines.into()
    })
}

/// The client's state, even after a thread panicked holding it: what it
/// guards stays usable, and panicking again would take the editor down
/// with one language server.
fn lock(state: &Mutex<State>) -> MutexGuard<'_, State> {
    state.lock().unwrap_or_else(|error| error.into_inner())
}

fn fail(state: &Mutex<State>, why: String) {
    let mut state = lock(state);
    state.failure = Some(why);
    if let Some(sink) = &state.on_failure {
        let _ = sink.unbounded_send(());
    }
}

fn die(state: &Mutex<State>) {
    let mut state = lock(state);
    state.phase = Phase::Dead;
    for (_, pending) in state.pending.drain() {
        let _ = pending.send(Err("the language server exited".into()));
    }
}
