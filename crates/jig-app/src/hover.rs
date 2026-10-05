//! What a language server says about the symbol under the pointer
//! (`textDocument/hover`) and the call being typed
//! (`textDocument/signatureHelp`), read into what Jig shows.
//!
//! Positions here are byte offsets; the editor's own are worked out by the
//! caller.

use std::ops::Range;

use serde_json::Value;

/// How far back from the cursor to look for the call it's in.
const MAX_CALL_SCAN_BYTES: usize = 8 * 1024;

/// A hover's contents as Markdown: `MarkupContent`, a `MarkedString` or an
/// array of them. `None` when there's nothing to show.
pub fn markdown(contents: &Value) -> Option<String> {
    let text = match contents {
        Value::Array(items) => items
            .iter()
            .filter_map(markdown)
            .collect::<Vec<_>>()
            .join("\n\n"),
        // A `MarkedString` is Markdown already.
        Value::String(text) => text.clone(),
        Value::Object(object) => {
            let value = object.get("value").and_then(Value::as_str).unwrap_or("");
            match (object.get("kind"), object.get("language")) {
                (Some(kind), _) if kind == "plaintext" => escape(value),
                (Some(_), _) => value.to_string(),
                // `{language, value}`: a code block.
                (None, Some(language)) => format!(
                    "```{}\n{}\n```",
                    language.as_str().unwrap_or(""),
                    value.trim_end()
                ),
                (None, None) => value.to_string(),
            }
        }
        _ => String::new(),
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// Plain text as Markdown that reads the same: punctuation escaped, line
/// breaks kept.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => out.push_str("  \n"),
            c if c.is_ascii_punctuation() => {
                out.push('\\');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out
}

/// The signature to show for a call, from a `SignatureHelp`.
#[derive(Clone, Debug, PartialEq)]
pub struct Signature {
    pub label: String,
    /// The active parameter, in `label`.
    pub active: Option<Range<usize>>,
    /// The active parameter's documentation, else the signature's, as plain
    /// text.
    pub documentation: Option<String>,
    /// Which of how many overloads this is, when there's more than one.
    pub overload: Option<(usize, usize)>,
}

/// The active signature of a `SignatureHelp`, with its active parameter.
/// `None` for no signature, as servers answer outside a call.
pub fn signature(help: &Value) -> Option<Signature> {
    let signatures = help.get("signatures")?.as_array()?;
    if signatures.is_empty() {
        return None;
    }
    // Out of range means the first, the protocol says.
    let chosen_ix = index(help.get("activeSignature"))
        .filter(|&ix| ix < signatures.len())
        .unwrap_or(0);
    let chosen = &signatures[chosen_ix];
    let label = chosen.get("label")?.as_str()?.to_string();
    let parameters = chosen
        .get("parameters")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let ranges = parameter_ranges(&label, parameters);
    // The signature's own wins over the help's.
    let active_ix = match chosen.get("activeParameter") {
        Some(Value::Null) | None => index(help.get("activeParameter")).or(Some(0)),
        given => index(given),
    }
    .filter(|&ix| ix < parameters.len());
    let active = active_ix.and_then(|ix| ranges.get(ix).cloned().flatten());
    let documentation = active_ix
        .and_then(|ix| plain(parameters[ix].get("documentation")?))
        .or_else(|| plain(chosen.get("documentation")?));
    Some(Signature {
        label,
        active,
        documentation,
        overload: (signatures.len() > 1).then_some((chosen_ix + 1, signatures.len())),
    })
}

fn index(value: Option<&Value>) -> Option<usize> {
    value?.as_u64().map(|ix| ix as usize)
}

/// Where each parameter is in `label`: given as `[start, end]` in UTF-16
/// units, or as its text, found in order after the opening parenthesis.
fn parameter_ranges(label: &str, parameters: &[Value]) -> Vec<Option<Range<usize>>> {
    let mut from = label.find('(').map_or(0, |ix| ix + 1);
    parameters
        .iter()
        .map(|parameter| {
            let range = match parameter.get("label")? {
                Value::String(text) if !text.is_empty() => {
                    let start = from + label[from..].find(text.as_str())?;
                    start..start + text.len()
                }
                Value::Array(bounds) => {
                    let start = utf16_to_byte(label, index(bounds.first())?)?;
                    let end = utf16_to_byte(label, index(bounds.get(1))?)?;
                    (start <= end).then_some(start..end)?
                }
                _ => return None,
            };
            from = range.end;
            Some(range)
        })
        .collect()
}

fn utf16_to_byte(text: &str, units: usize) -> Option<usize> {
    let mut count = 0;
    for (ix, c) in text.char_indices() {
        if count >= units {
            return Some(ix);
        }
        count += c.len_utf16();
    }
    (count >= units).then_some(text.len())
}

/// Documentation, a string or `MarkupContent`, as one trimmed paragraph.
fn plain(documentation: &Value) -> Option<String> {
    let text = match documentation {
        Value::String(text) => text.as_str(),
        object => object.get("value")?.as_str()?,
    };
    let first = text.trim().split("\n\n").next()?.trim();
    (!first.is_empty()).then(|| first.to_string())
}

/// Where the call the cursor is in opens: the offset of its unmatched `(`.
/// `None` outside a call, or inside a block or list within one. Strings and
/// comments aren't told apart; servers have the last word.
pub fn enclosing_call(text: &str, cursor: usize) -> Option<usize> {
    let cursor = cursor.min(text.len());
    let start = cursor.saturating_sub(MAX_CALL_SCAN_BYTES);
    let mut depth = 0usize;
    for (ix, b) in text.as_bytes()[start..cursor].iter().enumerate().rev() {
        match b {
            b')' | b']' | b'}' => depth += 1,
            b'(' if depth == 0 => return Some(start + ix),
            b'[' | b'{' if depth == 0 => return None,
            b'(' | b'[' | b'{' => depth -= 1,
            // A statement ends before any call that's still open.
            b';' if depth == 0 => return None,
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn markup_content() {
        let markdown_kind = json!({"kind": "markdown", "value": "```rust\nfn a()\n```\n\nDocs."});
        assert_eq!(
            markdown(&markdown_kind).unwrap(),
            "```rust\nfn a()\n```\n\nDocs."
        );
        // Plain text keeps its punctuation and lines.
        let plaintext = json!({"kind": "plaintext", "value": "a *b*\nc"});
        assert_eq!(markdown(&plaintext).unwrap(), "a \\*b\\*  \nc");
        assert_eq!(markdown(&json!({"kind": "markdown", "value": "  "})), None);
    }

    #[test]
    fn marked_strings_and_arrays() {
        assert_eq!(markdown(&json!("**bold**")).unwrap(), "**bold**");
        let code = json!({"language": "python", "value": "def f(x): ...\n"});
        assert_eq!(markdown(&code).unwrap(), "```python\ndef f(x): ...\n```");
        let both = json!([code, "", "Returns nothing."]);
        assert_eq!(
            markdown(&both).unwrap(),
            "```python\ndef f(x): ...\n```\n\nReturns nothing."
        );
        assert_eq!(markdown(&json!([])), None);
        assert_eq!(markdown(&Value::Null), None);
    }

    #[test]
    fn active_signature_and_parameter() {
        let help = json!({
            "signatures": [
                {"label": "f()"},
                {
                    "label": "f(a: u32, b: &str)",
                    "documentation": "Does f.",
                    "parameters": [
                        {"label": "a: u32", "documentation": {"kind": "markdown", "value": "The a.\n\nMore."}},
                        {"label": "b: &str"},
                    ],
                },
            ],
            "activeSignature": 1,
            "activeParameter": 0,
        });
        let found = signature(&help).unwrap();
        assert_eq!(&found.label[found.active.clone().unwrap()], "a: u32");
        assert_eq!(found.documentation.as_deref(), Some("The a."));
        assert_eq!(found.overload, Some((2, 2)));

        // The signature's own active parameter wins; without docs of its
        // own, the signature's show.
        let mut help = help;
        help["signatures"][1]["activeParameter"] = json!(1);
        let found = signature(&help).unwrap();
        assert_eq!(&found.label[found.active.unwrap()], "b: &str");
        assert_eq!(found.documentation.as_deref(), Some("Does f."));

        // Out of range: the first signature, no parameter.
        help["activeSignature"] = json!(7);
        help["activeParameter"] = json!(3);
        let found = signature(&help).unwrap();
        assert_eq!(found.label, "f()");
        assert_eq!(found.active, None);

        assert_eq!(signature(&json!({"signatures": []})), None);
        assert_eq!(signature(&Value::Null), None);
    }

    #[test]
    fn parameters_by_offset_and_repeated_text() {
        // `[start, end]` in UTF-16 units, after a non-ASCII name.
        let help = json!({
            "signatures": [{
                "label": "é(x: i32, y: i32)",
                "parameters": [{"label": [2, 8]}, {"label": [10, 16]}],
                "activeParameter": 1,
            }],
        });
        let found = signature(&help).unwrap();
        assert_eq!(&found.label[found.active.unwrap()], "y: i32");

        // The same text twice: each parameter is found after the last.
        let help = json!({
            "signatures": [{
                "label": "max(a, a)",
                "parameters": [{"label": "a"}, {"label": "a"}],
            }],
            "activeParameter": 1,
        });
        let found = signature(&help).unwrap();
        assert_eq!(found.active, Some(7..8));
    }

    #[test]
    fn finds_the_call_the_cursor_is_in() {
        let text = "let x = f(a, g(b), [c, d";
        let f = text.find("f(").unwrap() + 1;
        let g = text.find("g(").unwrap() + 1;
        assert_eq!(enclosing_call(text, text.find(", g").unwrap()), Some(f));
        assert_eq!(enclosing_call(text, text.find("b)").unwrap()), Some(g));
        assert_eq!(enclosing_call(text, text.find("),").unwrap() + 2), Some(f));
        // In a list inside the call, or before it.
        assert_eq!(enclosing_call(text, text.len()), None);
        assert_eq!(enclosing_call(text, 4), None);
        assert_eq!(enclosing_call("f(a); g", 7), None);
        assert_eq!(enclosing_call("f(a)", 4), None);
    }
}
