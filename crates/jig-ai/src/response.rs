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

/// The tool an answer comes back through: a message and nothing else, so
/// there's no code for the model to retype, or change while retyping it.
pub const ANSWER_TOOL: &str = "answer";

/// What the model is asked to return.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyKind {
    /// The region's new text and a short message.
    Edit,
    /// An answer about the region, which stays as it is.
    Answer,
}

impl ReplyKind {
    /// The tool a provider that can force one makes the model call.
    pub fn tool(self) -> &'static str {
        match self {
            ReplyKind::Edit => REPLY_TOOL,
            ReplyKind::Answer => ANSWER_TOOL,
        }
    }

    pub fn tool_description(self) -> &'static str {
        match self {
            ReplyKind::Edit => "Apply the edit to the marked region.",
            ReplyKind::Answer => "Answer about the marked region.",
        }
    }

    /// JSON Schema of the tool's input: the same object the prompt asks
    /// for in text.
    pub fn schema(self) -> serde_json::Value {
        match self {
            ReplyKind::Edit => reply_schema(),
            ReplyKind::Answer => serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The answer, plain text, at most 60 words."
                    }
                },
                "required": ["message"]
            }),
        }
    }
}

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
                "description": "One short plain sentence, at most 20 words, saying what was changed."
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

#[derive(Deserialize)]
struct RawAnswer {
    message: String,
}

/// Parse an answer. The region comes back as it was, whatever the model
/// sent, so an answer never changes the buffer. Plain text, from a model
/// that ignored the format, is taken as the answer: nothing can go wrong
/// with it.
pub fn parse_answer(raw: &str, original: &str) -> Result<Reply> {
    let from_json = raw.find('{').and_then(|start| {
        serde_json::Deserializer::from_str(&raw[start..])
            .into_iter::<RawAnswer>()
            .next()?
            .ok()
    });
    let message = match from_json {
        Some(answer) => answer.message,
        None if raw.contains("\"message\"") => {
            bail!("The model's answer didn't match the expected format.")
        }
        None => raw.to_string(),
    };
    let message = message.trim().to_string();
    if message.is_empty() {
        bail!("The model returned an empty answer.");
    }
    Ok(Reply {
        replace: original.to_string(),
        message,
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

    #[test]
    fn an_answer_keeps_the_region_as_it_was() {
        let reply = parse_answer(r#"{"message": " Sorts by score. "}"#, "code").unwrap();
        assert_eq!(reply.replace, "code");
        assert_eq!(reply.message, "Sorts by score.");
        // Code sent anyway is ignored, so the buffer can't change.
        let reply = parse_answer(r#"{"replace": "other", "message": "m"}"#, "code").unwrap();
        assert_eq!(reply.replace, "code");
        // Plain text is the answer.
        assert_eq!(parse_answer("It sorts.", "c").unwrap().message, "It sorts.");
        assert!(parse_answer(r#"{"message": ""}"#, "c").is_err());
        assert!(parse_answer(r#"{"message": 3}"#, "c").is_err());
    }

    #[test]
    fn each_kind_has_its_own_tool() {
        assert_eq!(ReplyKind::Edit.tool(), REPLY_TOOL);
        let answer = ReplyKind::Answer.schema();
        assert_eq!(answer["required"], serde_json::json!(["message"]));
        assert!(answer["properties"].get("replace").is_none());
    }
}
