//! Parsing the model's reply. Anything that doesn't fit the contract is an
//! error, and an error never changes the buffer.

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::prompt::{CURSOR, SELECTION_END, SELECTION_START};

pub const MAX_MESSAGE_WORDS: usize = 20;

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
        message: cap_words(&reply.message),
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

fn cap_words(message: &str) -> String {
    let words: Vec<&str> = message.split_whitespace().collect();
    if words.len() <= MAX_MESSAGE_WORDS {
        words.join(" ")
    } else {
        format!("{}…", words[..MAX_MESSAGE_WORDS].join(" "))
    }
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
    fn message_is_capped() {
        let long = (1..=30)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");
        let reply = parse(&format!(r#"{{"replace": "x", "message": "{long}"}}"#), "y").unwrap();
        assert_eq!(reply.message.split_whitespace().count(), MAX_MESSAGE_WORDS);
        assert!(reply.message.ends_with("w20…"));
    }

    #[test]
    fn trailing_newline_follows_original() {
        assert_eq!(parse(r#"{"replace": "a\n"}"#, "b").unwrap().replace, "a");
        assert_eq!(parse(r#"{"replace": "a"}"#, "b\n").unwrap().replace, "a\n");
        // Insertions (empty original) are left alone.
        assert_eq!(parse(r#"{"replace": "a\n"}"#, "").unwrap().replace, "a\n");
    }
}
