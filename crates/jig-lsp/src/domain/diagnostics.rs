//! The problems a server reports (`textDocument/publishDiagnostics`), as
//! read from its notification.

use serde_json::Value;

use super::positions::Encoding;

/// One report: every problem a server sees in one file right now.
#[derive(Clone, Debug, PartialEq)]
pub struct Published {
    /// The file, as `diagnostics::path_key` names it.
    pub key: String,
    /// The version of the text the server looked at, if it said.
    pub version: Option<i32>,
    pub items: Vec<lsp_types::Diagnostic>,
    /// How the server counts columns in the items' ranges.
    pub encoding: Encoding,
}

/// A `publishDiagnostics` notification's params, or `None` if they don't
/// make sense, kept under `key` of its URI. Items that don't parse are
/// skipped, not the whole report.
pub fn parse(
    params: &Value,
    encoding: Encoding,
    key: impl FnOnce(&str) -> String,
) -> Option<Published> {
    let uri = params.get("uri")?.as_str()?;
    let items = params
        .get("diagnostics")?
        .as_array()?
        .iter()
        .filter_map(|item| serde_json::from_value(item.clone()).ok())
        .collect();
    Some(Published {
        key: key(uri),
        version: params
            .get("version")
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok()),
        items,
        encoding,
    })
}
