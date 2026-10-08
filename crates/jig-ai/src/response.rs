//! Parsing the model's reply. Anything that doesn't fit the contract is an
//! error, and an error never changes the buffer.

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::prompt::{CURSOR, SELECTION_END, SELECTION_START};

/// The reply as a tool the model must call, for providers that can force
/// one. A forced tool call always comes back as structured input, so the
/// model can't drift into prose the way it sometimes does with a big file
/// or an instruction that doesn't fit the selection.
pub const REPLY_TOOL: &str = "apply_edit";

/// JSON Schema of [`REPLY_TOOL`]'s input: the same object the prompt asks
/// for in text.
pub fn reply_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "replace": {
                "type": "string",
                "description": "The complete new text for the marked region, or the text to insert at the cursor."
            },
            "message": {
                "type": "string",
                "description": "One short plain sentence, at most 20 words."
            }
        },
        "required": ["replace", "message"]
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// The new text for the target range.
    pub replace: String,
    /// A short note for the reply bubble.
    pub message: String,
}

#[derive(Deserialize)]
struct RawReply {
    replace: String,
    #[serde(default)]
    message: String,
}

/// Parse `raw` model output. `original` is the text being replaced; it is
/// used to undo common model habits like adding a trailing newline.
pub fn parse(raw: &str, original: &str) -> Result<Reply> {
    let start = raw.find('{').context("The model didn't return JSON.")?;
    // Read the first complete JSON value, ignoring any prose after it.
    let mut stream = serde_json::Deserializer::from_str(&raw[start..]).into_iter::<RawReply>();
    let reply = match stream.next() {
        Some(Ok(reply)) => reply,
        Some(Err(error)) => bail!("The model's reply didn't match the expected format ({error})."),
        None => bail!("The model didn't return JSON."),
    };
    if [SELECTION_START, SELECTION_END, CURSOR]
        .iter()
        .any(|marker| reply.replace.contains(marker))
    {
        bail!("The model returned the region markers; nothing was changed.");
    }
    Ok(Reply {
        replace: match_trailing_newline(reply.replace, original),
        message: reply.message.trim().to_string(),
    })
}

/// Models often add or drop a final newline. Make the replacement end the
/// way the original did.
fn match_trailing_newline(mut replace: String, original: &str) -> String {
    if original.is_empty() {
        return replace;
    }
    let had = original.ends_with('\n');
    if had && !replace.ends_with('\n') {
        replace.push('\n');
    } else if !had {
        while replace.ends_with('\n') {
            replace.pop();
        }
    }
    replace
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_json() {
        let reply = parse(
            r#"{"replace": "fn a() {}", "message": "Done."}"#,
            "fn a(){}",
        )
        .unwrap();
        assert_eq!(
            reply,
            Reply {
                replace: "fn a() {}".into(),
                message: "Done.".into()
            }
        );
    }

    #[test]
    fn fenced_json_with_prose() {
        let raw = "Sure! Here you go:\n```json\n{\"replace\": \"x\", \"message\": \"Renamed.\"}\n```\nLet me know {if} more.";
        assert_eq!(parse(raw, "y").unwrap().replace, "x");
    }

    #[test]
    fn missing_message_is_allowed() {
        assert_eq!(parse(r#"{"replace": "x"}"#, "y").unwrap().message, "");
    }

    #[test]
    fn malformed_is_an_error() {
        assert!(parse("I can't do that.", "y").is_err());
        assert!(parse(r#"{"replace": 3}"#, "y").is_err());
        assert!(parse(r#"{"message": "no code"}"#, "y").is_err());
        assert!(parse(r#"{"replace": "x", "#, "y").is_err());
    }

    #[test]
    fn markers_are_rejected() {
        assert!(parse(r#"{"replace": "<<<SELECTION>>>x<<<END>>>"}"#, "y").is_err());
    }

    #[test]
    fn long_message_is_kept_whole() {
        let long = (1..=30)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let reply = parse(&format!(r#"{{"replace": "x", "message": "{long}"}}"#), "y").unwrap();
        assert_eq!(reply.message, long);
    }

    #[test]
    fn trailing_newline_follows_original() {
        assert_eq!(parse(r#"{"replace": "a\n"}"#, "b").unwrap().replace, "a");
        assert_eq!(parse(r#"{"replace": "a"}"#, "b\n").unwrap().replace, "a\n");
        // Insertions (empty original) are left alone.
        assert_eq!(parse(r#"{"replace": "a\n"}"#, "").unwrap().replace, "a\n");
    }
}
