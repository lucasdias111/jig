//! `file:` URIs, as servers want them and send them.

use std::path::{Path, PathBuf};

/// A `file:` URI for `path`, made absolute first: a relative path makes no
/// sense to a server.
pub fn file_uri(path: &Path) -> String {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let path = path.to_string_lossy();
    // `C:\a\b.rs` is `file:///C:/a/b.rs`, the drive's colon kept as is.
    if cfg!(windows) {
        let path = path.strip_prefix(r"\\?\").unwrap_or(&path);
        let path = format!("/{}", path.replace('\\', "/"));
        let mut encoded = encode(&path);
        if encoded.get(2..5) == Some("%3A") {
            encoded.replace_range(2..5, ":");
        }
        return format!("file://{encoded}");
    }
    format!("file://{}", encode(&path))
}

/// The path of a `file:` URI.
pub fn uri_path(uri: &str) -> Option<PathBuf> {
    let path = uri.strip_prefix("file://")?;
    // `file://localhost/…` is allowed, if rare.
    let path = path.strip_prefix("localhost").unwrap_or(path);
    let path = decode(path);
    // `/C:/a/b.rs` on Windows, where the drive comes first.
    if cfg!(windows)
        && let Some(rest) = path.strip_prefix('/')
        && rest.as_bytes().get(1) == Some(&b':')
    {
        return Some(PathBuf::from(rest));
    }
    Some(PathBuf::from(path))
}

/// Percent-encode everything but unreserved characters and `/`.
pub fn encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%')
            .then(|| text.get(i + 1..i + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}
