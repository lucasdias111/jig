//! The request sent to the model.

use std::ops::Range;

pub const SELECTION_START: &str = "<<<SELECTION>>>";
pub const SELECTION_END: &str = "<<<END>>>";
pub const CURSOR: &str = "<<<CURSOR>>>";

const SYSTEM: &str = r#"You are Jig, a precise code-editing tool inside a text editor. You are not a chat assistant.

You receive a file in which the region to change is marked:
- <<<SELECTION>>> ... <<<END>>> surrounds code to replace, or
- <<<CURSOR>>> marks where new code is inserted.

Apply the instruction to that region only. A Note, when present, is the user's detail for this run (a name, a constraint, a choice); follow it. Project rules, when present, are the project's conventions; follow them unless the instruction or Note says otherwise. Reply with exactly one JSON object and nothing else:
{"replace": "<new text for the region>", "message": "<one short sentence>"}

Rules for "replace":
- It is the complete new text for the marked region (or the text to insert at the cursor). It replaces the region verbatim.
- Never include the markers or any code outside the region.
- Match the file's existing indentation and style. No markdown fences.
- If the instruction is a question or asks for an explanation, return the region unchanged and answer in "message".

Rules for "message": at most 20 words, plain text, no greetings, no follow-up questions."#;

/// Added to the system prompt when the command may explore the project.
pub const EXPLORE: &str = "You can call read-only tools to look at other files in the project (paths are relative to the project root). Use them only when the instruction needs something that isn't in the file, such as a type, a function signature or a convention defined elsewhere. The file you are editing is already included in full above; don't read it again. Look at as little as you need, then reply with the JSON object described above.";

/// Everything the model needs for one command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptRequest {
    pub instruction: String,
    /// The user's note for this run, refining the instruction.
    pub comment: Option<String>,
    /// The project's conventions, from its `JIG.md`.
    pub project_rules: Option<String>,
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
    let note = request
        .comment
        .as_deref()
        .map(|comment| format!("Note: {comment}\n"))
        .unwrap_or_default();
    let rules = request
        .project_rules
        .as_deref()
        .map(|rules| format!("<project_rules>\n{}\n</project_rules>\n\n", rules.trim()))
        .unwrap_or_default();
    let user = format!(
        "{rules}Language: {}\nFile: {file}\nInstruction: {}\n{note}\n<file>\n{marked}\n</file>",
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
            comment: None,
            project_rules: None,
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
    fn includes_the_note() {
        let mut with_note = request(10..19);
        with_note.comment = Some("Entity: User".into());
        let (_, user) = build(&with_note);
        assert!(user.contains("Instruction: Add docs\nNote: Entity: User\n"));
        let (_, user) = build(&request(10..19));
        assert!(!user.contains("Note:"));
    }

    #[test]
    fn includes_project_rules_first() {
        let mut with_rules = request(10..19);
        with_rules.project_rules = Some("Use thiserror for errors.\n".into());
        let (system, user) = build(&with_rules);
        assert!(system.contains("Project rules"));
        assert!(user.starts_with(
            "<project_rules>\nUse thiserror for errors.\n</project_rules>\n\nLanguage: rust"
        ));
        assert!(!build(&request(10..19)).1.contains("project_rules"));
    }

    #[test]
    fn marks_cursor() {
        let (_, user) = build(&request(10..10));
        assert!(user.contains("fn a() {}\n<<<CURSOR>>>fn b() {}"));
        assert!(!user.contains(SELECTION_START));
    }
}
