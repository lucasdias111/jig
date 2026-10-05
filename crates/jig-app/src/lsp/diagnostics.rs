//! The problems language servers report (`textDocument/publishDiagnostics`),
//! kept per file whether or not it's open, so a file shows its problems as
//! soon as it opens. Each report replaces the file's last one; an empty one
//! clears it.

use std::collections::HashMap;
use std::ops::Range;
use std::path::Path;

use gpui_kit::base::input::Diagnostic;
use gpui_kit::{App, Global};
use serde_json::Value;

use super::Encoding;

/// One report: every problem a server sees in one file right now.
#[derive(Clone, Debug, PartialEq)]
pub struct Published {
    /// The file, as [`path_key`] names it.
    pub key: String,
    /// The version of the text the server looked at, if it said.
    pub version: Option<i32>,
    pub items: Vec<lsp_types::Diagnostic>,
    /// How the server counts columns in the items' ranges.
    pub encoding: Encoding,
}

/// A `publishDiagnostics` notification's params, or `None` if they don't
/// make sense. Items that don't parse are skipped, not the whole report.
pub fn parse(params: &Value, encoding: Encoding) -> Option<Published> {
    let uri = params.get("uri")?.as_str()?;
    let items = params
        .get("diagnostics")?
        .as_array()?
        .iter()
        .filter_map(|item| serde_json::from_value(item.clone()).ok())
        .collect();
    Some(Published {
        key: uri_key(uri),
        version: params
            .get("version")
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok()),
        items,
        encoding,
    })
}

/// The name a file's problems are kept under. Servers don't write URIs the
/// way Jig does (`file:///c%3A/…` for `C:\…` on Windows, symlinks resolved
/// or not), so both sides go through the file's real path.
pub fn path_key(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let key = super::file_uri(&path);
    // Windows doesn't tell `C:\A.rs` from `c:\a.rs`.
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

/// [`path_key`] for a URI a server sent.
pub fn uri_key(uri: &str) -> String {
    match super::uri_path(uri) {
        Some(path) => path_key(&path),
        // Not a file: kept as it is, though no tab will ask for it.
        None => uri.to_string(),
    }
}

/// Where in `text` each problem is, as the editor wants them: byte ranges,
/// clamped to the text.
pub fn entries(text: &str, published: &Published) -> Vec<(Range<usize>, Diagnostic)> {
    published
        .items
        .iter()
        .map(|item| {
            let range = super::range(text, item.range, published.encoding);
            (range.start.min(range.end)..range.end, item.clone().into())
        })
        .collect()
}

/// Every file's problems, as last reported.
#[derive(Default)]
pub struct Diagnostics {
    files: HashMap<String, Stored>,
    /// Counts reports, so a tab can tell whether its file's changed.
    reports: u64,
}

pub struct Stored {
    pub published: Published,
    /// Which report this was, so a tab can tell it has the latest.
    pub revision: u64,
}

impl Global for Diagnostics {}

impl Diagnostics {
    pub fn get(&self, key: &str) -> Option<&Stored> {
        self.files.get(key)
    }
}

/// Keep `published`, replacing the file's last report. Observers of
/// [`Diagnostics`] hear about it.
pub fn publish(published: Published, cx: &mut App) {
    let store = cx.default_global::<Diagnostics>();
    if published.items.is_empty() {
        store.files.remove(&published.key);
    } else {
        store.reports += 1;
        let revision = store.reports;
        store.files.insert(
            published.key.clone(),
            Stored {
                published,
                revision,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_a_report() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a b.rs");
        std::fs::write(&file, "").unwrap();
        let uri = super::super::file_uri(&file);
        let params = json!({
            "uri": uri,
            "version": 3,
            "diagnostics": [
                {
                    "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 9}},
                    "severity": 1,
                    "source": "rustc",
                    "code": "E0308",
                    "message": "mismatched types",
                },
                {"message": "no range, so skipped"},
                {
                    "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 1}},
                    "message": "no severity",
                },
            ],
        });
        let published = parse(&params, Encoding::Utf16).unwrap();
        assert_eq!(published.key, path_key(&file));
        assert_eq!(published.version, Some(3));
        assert_eq!(published.items.len(), 2);
        assert_eq!(published.items[0].message, "mismatched types");
        assert_eq!(
            published.items[0].severity,
            Some(lsp_types::DiagnosticSeverity::ERROR)
        );
        assert!(parse(&json!({"diagnostics": []}), Encoding::Utf16).is_none());
    }

    #[test]
    fn keys_match_however_the_uri_is_written() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        let file = dir.path().join("src/naïve.rs");
        std::fs::write(&file, "").unwrap();
        let key = path_key(&file);
        // Through `..`, or with the real path a server resolved itself
        // (on macOS the temp folder is behind a symlink).
        let roundabout = dir.path().join("src/../src/naïve.rs");
        assert_eq!(uri_key(&super::super::file_uri(&roundabout)), key);
        let real = file.canonicalize().unwrap();
        assert_eq!(uri_key(&super::super::file_uri(&real)), key);
        // Encoded more than Jig would.
        let encoded = super::super::file_uri(&real).replace('/', "%2F");
        let encoded = encoded.replacen("file:%2F%2F", "file://", 1);
        assert_eq!(uri_key(&encoded), key);
    }

    #[test]
    fn utf16_columns_become_byte_ranges() {
        let text = "let s = \"😀\"; bad\nnext\n";
        let item = |start: u32, end: u32| lsp_types::Diagnostic {
            range: lsp_types::Range::new(
                lsp_types::Position::new(0, start),
                lsp_types::Position::new(0, end),
            ),
            message: "x".into(),
            ..Default::default()
        };
        // `bad` starts after the emoji, two UTF-16 units but four bytes.
        let published = Published {
            key: String::new(),
            version: None,
            items: vec![item(14, 17), item(30, 99)],
            encoding: Encoding::Utf16,
        };
        let ranges: Vec<_> = entries(text, &published)
            .into_iter()
            .map(|(range, _)| range)
            .collect();
        let bad = text.find("bad").unwrap();
        // Past the end of the line stays on it.
        let line_end = text.find('\n').unwrap();
        assert_eq!(ranges, [bad..bad + 3, line_end..line_end]);
        assert_eq!(&text[ranges[0].clone()], "bad");

        let utf8 = Published {
            items: vec![item(16, 19)],
            encoding: Encoding::Utf8,
            ..published
        };
        assert_eq!(entries(text, &utf8)[0].0, bad..bad + 3);
    }
}
