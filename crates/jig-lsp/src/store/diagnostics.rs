//! The names problems are kept under: each file's real path, as a URI.

use std::path::Path;

use serde_json::Value;

use crate::domain::diagnostics::{self, Published};
use crate::domain::positions::Encoding;
use crate::domain::uri::{file_uri, uri_path};

/// A `publishDiagnostics` notification's params, or `None` if they don't
/// make sense, kept under the file's [`uri_key`].
pub fn parse(params: &Value, encoding: Encoding) -> Option<Published> {
    diagnostics::parse(params, encoding, uri_key)
}

/// The name a file's problems are kept under. Servers don't write URIs the
/// way Jig does (`file:///c%3A/…` for `C:\…` on Windows, symlinks resolved
/// or not), so both sides go through the file's real path.
pub fn path_key(path: &Path) -> String {
    let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let key = file_uri(&path);
    // Windows doesn't tell `C:\A.rs` from `c:\a.rs`.
    if cfg!(windows) {
        key.to_lowercase()
    } else {
        key
    }
}

/// [`path_key`] for a URI a server sent.
pub fn uri_key(uri: &str) -> String {
    match uri_path(uri) {
        Some(path) => path_key(&path),
        // Not a file: kept as it is, though no tab will ask for it.
        None => uri.to_string(),
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
        let uri = file_uri(&file);
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
        assert_eq!(uri_key(&file_uri(&roundabout)), key);
        let real = file.canonicalize().unwrap();
        assert_eq!(uri_key(&file_uri(&real)), key);
        // Encoded more than Jig would.
        let encoded = file_uri(&real).replace('/', "%2F");
        let encoded = encoded.replacen("file:%2F%2F", "file://", 1);
        assert_eq!(uri_key(&encoded), key);
    }
}
