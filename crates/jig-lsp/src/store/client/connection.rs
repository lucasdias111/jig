//! Talking to one server: writing on a thread of its own, and reading its
//! messages until it hangs up.

use std::io::{BufReader, Read, Write};
use std::path::Path;
use std::sync::{Arc, Mutex, mpsc};

use serde_json::{Value, json};

use super::{Phase, State, die, lock};
use crate::domain::protocol::{
    Capabilities, INITIALIZE_ID, answer_server, frame, initialize, read_message, response,
};
use crate::domain::uri::file_uri;
use crate::store::diagnostics::{parse, uri_key};

/// Talk to a server over `reader` and `writer` until it hangs up.
pub(super) fn connect(
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

    let init_options = lock(state).init_options.clone();
    let root_uri = file_uri(root);
    let name = root
        .file_name()
        .map_or_else(|| "root".into(), |name| name.to_string_lossy().into_owned());
    let process_id = std::process::id();
    let _ = tx.send(frame(&initialize(
        root,
        &root_uri,
        &name,
        process_id,
        init_options,
    )));

    let mut reader = BufReader::new(reader);
    while let Some(message) = read_message(&mut reader) {
        let id = message.get("id").cloned();
        let method = message.get("method").and_then(Value::as_str);
        match (id, method) {
            // The server asking us something: answer so it doesn't wait.
            (Some(id), Some(method)) => {
                let result = answer_server(method, &message["params"], &root_uri, &name);
                let _ = tx.send(frame(
                    &json!({"jsonrpc": "2.0", "id": id, "result": result}),
                ));
            }
            (Some(id), None) => {
                let response = response(&message);
                if id.as_i64() == Some(INITIALIZE_ID) {
                    let Ok(result) = response else {
                        break;
                    };
                    let capabilities = Capabilities::from_result(&result);
                    let mut state = lock(state);
                    state.encoding = capabilities.encoding;
                    state.completion_triggers = capabilities.completion_triggers;
                    state.formatting = capabilities.formatting;
                    state.started = true;
                    let _ = tx.send(frame(
                        &json!({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
                    ));
                    if let Phase::Starting(queue) =
                        std::mem::replace(&mut state.phase, Phase::Ready(tx.clone()))
                    {
                        for bytes in queue {
                            let _ = tx.send(bytes);
                        }
                    }
                } else if let Some(pending) =
                    id.as_i64().and_then(|id| lock(state).pending.remove(&id))
                {
                    let _ = pending.send(response);
                }
            }
            // jdtls says when it has loaded the project, which its
            // commands need first.
            (None, Some("language/status")) if message["params"]["type"] == "ServiceReady" => {
                lock(state).service_ready = true;
            }
            (None, Some("textDocument/publishDiagnostics")) => {
                let state = lock(state);
                if let Some(published) = parse(&message["params"], state.encoding)
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
fn is_stale(state: &State, published: &crate::domain::diagnostics::Published) -> bool {
    let Some(version) = published.version else {
        return false;
    };
    state
        .documents
        .iter()
        .find(|(uri, _)| uri_key(uri) == published.key)
        .is_some_and(|(_, document)| document.version > version)
}
