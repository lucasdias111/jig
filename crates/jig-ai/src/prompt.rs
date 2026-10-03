//! The request sent to the model.

use std::ops::Range;

pub const SELECTION_START: &str = "<<<SELECTION>>>";
pub const SELECTION_END: &str = "<<<END>>>";
pub const CURSOR: &str = "<<<CURSOR>>>";

const SYSTEM: &str = r#"You are Jig, a precise code-editing tool inside a text editor. You are not a chat assistant.

You receive a file in which the region to change is marked:
- <<<SELECTION>>> ... <<<END>>> surrounds code to replace, or
- <<<CURSOR>>> marks where new code is inserted.

Apply the instruction to that region only. Reply with exactly one JSON object and nothing else:
{"replace": "<new text for the region>", "message": "<one short sentence>"}

Rules for "replace":
- It is the complete new text for the marked region (or the text to insert at the cursor). It replaces the region verbatim.
- Never include the markers or any code outside the region.
- Match the file's existing indentation and style. No markdown fences.
- If the instruction is a question or asks for an explanation, return the region unchanged and answer in "message".

Rules for "message": at most 20 words, plain text, no greetings, no follow-up questions."#;

/// Everything the model needs for one command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptRequest {
    pub instruction: String,
    pub language: String,
    pub file_name: Option<String>,
    pub text: String,
    /// The byte range the command replaces. Empty means insert at that offset.
    pub target: Range<usize>,
}

impl PromptRequest {
    pub fn target_text(&self) -> &str {
        &self.text[self.target.clone()]
    }
}

/// The system and user messages for `request`.
pub fn build(request: &PromptRequest) -> (String, String) {
    let text = &request.text;
    let range = request.target.clone();
    let marked = if range.is_empty() {
        format!("{}{CURSOR}{}", &text[..range.start], &text[range.start..])
    } else {
        format!(
            "{}{SELECTION_START}{}{SELECTION_END}{}",
            &text[..range.start],
            &text[range.clone()],
            &text[range.end..]
        )
    };
    let file = request.file_name.as_deref().unwrap_or("untitled");
    let user = format!(
        "Language: {}\nFile: {file}\nInstruction: {}\n\n<file>\n{marked}\n</file>",
        request.language, request.instruction
    );
    (SYSTEM.to_string(), user)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(target: Range<usize>) -> PromptRequest {
        PromptRequest {
            instruction: "Add docs".into(),
            language: "rust".into(),
            file_name: Some("lib.rs".into()),
            text: "fn a() {}\nfn b() {}\n".into(),
            target,
        }
    }

    #[test]
    fn marks_selection() {
        let (system, user) = build(&request(10..19));
        assert!(system.contains("\"replace\""));
        assert!(user.contains("Instruction: Add docs"));
        assert!(user.contains("fn a() {}\n<<<SELECTION>>>fn b() {}<<<END>>>\n"));
    }

    #[test]
    fn marks_cursor() {
        let (_, user) = build(&request(10..10));
        assert!(user.contains("fn a() {}\n<<<CURSOR>>>fn b() {}"));
        assert!(!user.contains(SELECTION_START));
    }
}
