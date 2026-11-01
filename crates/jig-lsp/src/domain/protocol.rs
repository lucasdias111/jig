//! The Language Server Protocol's messages, as Jig writes and reads them:
//! framing, `initialize` and what a server says it can do, the requests
//! servers send back, and the answers Jig asks for.

use std::io::BufRead;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::positions::Encoding;
use super::uri::uri_path;

/// The id `initialize` is sent with; requests count up from 1.
pub const INITIALIZE_ID: i64 = 0;

/// Which formatting requests a server answers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Formatting {
    pub document: bool,
    pub range: bool,
}

impl Formatting {
    /// From `initialize`'s capabilities, where each is `true` or options.
    pub fn from_capabilities(capabilities: &Value) -> Self {
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

/// What a server said it can do, in answer to `initialize`.
pub struct Capabilities {
    pub encoding: Encoding,
    /// What typed after a name asks for completions, such as `.`.
    pub completion_triggers: Vec<String>,
    pub formatting: Formatting,
}

impl Capabilities {
    /// From `initialize`'s result.
    pub fn from_result(result: &Value) -> Self {
        let capabilities = &result["capabilities"];
        let encoding = match capabilities["positionEncoding"].as_str() {
            Some("utf-8") => Encoding::Utf8,
            _ => Encoding::Utf16,
        };
        let completion_triggers = capabilities["completionProvider"]["triggerCharacters"]
            .as_array()
            .map(|triggers| {
                triggers
                    .iter()
                    .filter_map(|t| Some(t.as_str()?.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            encoding,
            completion_triggers,
            formatting: Formatting::from_capabilities(capabilities),
        }
    }
}

/// `message`, framed with its `Content-Length` for the wire.
pub fn frame(message: &Value) -> Vec<u8> {
    let body = message.to_string();
    format!("Content-Length: {}\r\n\r\n{body}", body.len()).into_bytes()
}

/// The `initialize` request for a server in `root`, whose URI is
/// `root_uri` and folder name `name`.
pub fn initialize(
    root: &Path,
    root_uri: &str,
    name: &str,
    process_id: u32,
    init_options: Value,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": INITIALIZE_ID,
        "method": "initialize",
        "params": {
            "processId": process_id,
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
                    "documentHighlight": {},
                    "formatting": {},
                    "rangeFormatting": {},
                    // Fixes with their edits in them; Jig runs no commands.
                    "codeAction": {
                        "codeActionLiteralSupport": {
                            "codeActionKind": {"valueSet": ["quickfix"]},
                        },
                        "isPreferredSupport": true,
                        "disabledSupport": true,
                    },
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
    })
}

/// The answer to a request a server sent, `method` with `params`, so it
/// doesn't wait: the workspace folder, and nothing for the rest.
pub fn answer_server(method: &str, params: &Value, root_uri: &str, name: &str) -> Value {
    match method {
        "workspace/configuration" => {
            let items = params["items"].as_array().map_or(0, Vec::len);
            Value::Array(vec![Value::Null; items])
        }
        "workspace/workspaceFolders" => json!([{"uri": root_uri, "name": name}]),
        _ => Value::Null,
    }
}

/// A response's result, or its error in words.
pub fn response(message: &Value) -> Result<Value, String> {
    match message.get("error") {
        Some(error) => Err(error["message"]
            .as_str()
            .unwrap_or("the language server failed")
            .to_string()),
        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
    }
}

/// What to say about `name` stopping before it started, having last
/// written `stderr`.
pub fn failure(name: &str, stderr: &[String]) -> String {
    let said = stderr.last().map(|line| {
        let line = line.trim();
        line.strip_prefix("error:")
            .unwrap_or(line)
            .trim()
            .to_string()
    });
    match said {
        Some(said) => format!("{name} didn't start: {said}"),
        None => format!("{name} stopped as it started."),
    }
}

/// One `Content-Length`-framed message, or `None` once the stream ends.
pub fn read_message(reader: &mut impl BufRead) -> Option<Value> {
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

/// A hash of `text`, so the same text isn't sent twice.
pub fn hash(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Where a definition or reference request pointed: a file and a range in
/// the server's positions.
pub fn locations(response: &Value) -> Vec<(PathBuf, lsp_types::Range)> {
    let items = match response {
        Value::Array(items) => items.clone(),
        Value::Null => Vec::new(),
        single => vec![single.clone()],
    };
    items
        .iter()
        .filter_map(|item| {
            // A `Location`, or a `LocationLink` pointing at its name.
            let (uri, range) = match item.get("targetUri") {
                Some(uri) => (uri, item.get("targetSelectionRange")?),
                None => (item.get("uri")?, item.get("range")?),
            };
            let path = uri_path(uri.as_str()?)?;
            let range = serde_json::from_value(range.clone()).ok()?;
            Some((path, range))
        })
        .collect()
}

pub fn text_document_position(uri: &str, position: lsp_types::Position) -> Value {
    json!({
        "textDocument": {"uri": uri},
        "position": position,
    })
}
