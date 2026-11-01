//! Debug adapters: the Debug Adapter Protocol, framed like LSP, over an
//! adapter's stdin and stdout (`lldb-dap`) or a TCP connection to an adapter
//! server (`js-debug`).
//!
//! Everything an adapter says, responses included, arrives as a
//! [`Message`] on one channel, in order, tagged with the id of the
//! connection it came on: one debug session can have several, as js-debug
//! runs the program in a child session of its own. The debug session is a
//! state machine over those messages, so nothing waits on a reply.

use std::io::{BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result};
use futures::channel::mpsc::UnboundedSender;
use jig_process::NoWindow as _;
use serde_json::{Value, json};

use jig_debuggers::AdapterCommand;

use crate::domain::messages::{Incoming, Message, frame, parse, read_message};

/// Where a client's messages go: `(connection id, message)`.
pub type Inbox = UnboundedSender<(usize, Message)>;

/// How long a starting adapter server gets to start listening.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Client {
    writer: mpsc::Sender<Vec<u8>>,
    next_seq: AtomicI64,
    child: Mutex<Option<Child>>,
}

impl Client {
    /// Start `adapter` in `cwd`, talking over its stdin and stdout.
    pub fn start(id: usize, adapter: &AdapterCommand, cwd: &Path, inbox: Inbox) -> Result<Self> {
        let mut child = Command::new(&adapter.program)
            .no_window()
            .args(&adapter.args)
            .current_dir(cwd)
            .env("PATH", jig_tools::search_path())
            .envs(adapter.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Couldn't start {}", adapter.program.display()))?;
        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;
        let client = Self::connect(id, stdout, stdin, inbox);
        *client.child.lock().unwrap() = Some(child);
        Ok(client)
    }

    /// Connect to an adapter server on `port`, retrying while it starts.
    /// Requests sent meanwhile wait, so the caller can carry on at once.
    pub fn connect_tcp(id: usize, port: u16, inbox: Inbox) -> Self {
        let (port_tx, port_rx) = mpsc::channel();
        let _ = port_tx.send(Ok(port));
        Self::connect_when(id, port_rx, inbox)
    }

    /// Connect once `port` says where, as when a language server starts
    /// the adapter; an error there ends the session with it.
    pub fn connect_when(
        id: usize,
        port: mpsc::Receiver<Result<u16, String>>,
        inbox: Inbox,
    ) -> Self {
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        let replies = tx.clone();
        std::thread::spawn(move || {
            let port = match port.recv() {
                Ok(Ok(port)) => port,
                Ok(Err(why)) => {
                    let _ = inbox.unbounded_send((id, Message::Failed(why)));
                    return;
                }
                Err(_) => {
                    let _ = inbox.unbounded_send((id, Message::Closed));
                    return;
                }
            };
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

/// An adapter that serves DAP on a TCP port, such as js-debug or Delve.
/// Dropping it stops it.
pub struct Server {
    child: Option<Child>,
    pub port: u16,
}

impl Server {
    /// Start `adapter`, whose arguments already name `port`, in `cwd`.
    pub fn start(adapter: &AdapterCommand, cwd: &Path, port: u16) -> Result<Self> {
        let child = Command::new(&adapter.program)
            .no_window()
            .args(&adapter.args)
            .current_dir(cwd)
            .env("PATH", jig_tools::search_path())
            .envs(adapter.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Couldn't start {}", adapter.program.display()))?;
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
pub fn free_port() -> Result<u16> {
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
        let parsed = match parse(&message) {
            Incoming::Message(parsed) => parsed,
            Incoming::Decline(reply) => {
                let _ = replies.send(frame(&reply));
                continue;
            }
            Incoming::Ignored => continue,
        };
        if inbox.unbounded_send((id, parsed)).is_err() {
            return;
        }
    }
    let _ = inbox.unbounded_send((id, Message::Closed));
}

/// Two connected sockets, standing in for a client and its adapter in
/// tests. Loopback TCP, since Unix socket pairs don't exist on Windows.
#[cfg(any(test, feature = "testing"))]
pub fn socket_pair() -> (std::net::TcpStream, std::net::TcpStream) {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let ours = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    let (theirs, _) = listener.accept().unwrap();
    (ours, theirs)
}
