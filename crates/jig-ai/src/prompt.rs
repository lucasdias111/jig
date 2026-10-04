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

/// Everything the model needs for one command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptRequest {
    pub instruction: String,
    /// The user's note for this run, refining the instruction.
    pub comment: Option<String>,
    /// The project's conventions, from its `AGENTS.md`.
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

/// Files longer than this are sent as a window around the region: a model
/// reading a huge file is slower and more likely to forget the reply format.
const MAX_FILE_BYTES: usize = 24 * 1024;

/// Repeated after the file, where the model reads it last.
const REMINDER: &str = "Reply with the JSON object only: {\"replace\": ..., \"message\": ...}. No prose before or after it.";

/// The part of `text` to send for `range`: all of it when it's small,
/// otherwise whole lines around `range`, about [`MAX_FILE_BYTES`] in all.
fn window(text: &str, range: &Range<usize>) -> Range<usize> {
    if text.len() <= MAX_FILE_BYTES {
        return 0..text.len();
    }
    let spare = MAX_FILE_BYTES.saturating_sub(range.len());
    let start = range.start.saturating_sub(spare / 2);
    let end = (range.end + spare - (range.start - start)).min(text.len());
    // Whole lines only.
    let start = match text[..start].rfind('\n') {
        Some(newline) if start > 0 => newline + 1,
        _ => 0,
    };
    let start = start.min(range.start);
    let end = text[end..]
        .find('\n')
        .map_or(text.len(), |newline| end + newline + 1);
    start..end.max(range.end)
}

/// The system and user messages for `request`.
pub fn build(request: &PromptRequest) -> (String, String) {
    let text = &request.text;
    let range = request.target.clone();
    let shown = window(text, &range);
    let above = text[..shown.start].matches('\n').count();
    let below = text[shown.end..].matches('\n').count();
    let (before, after) = (&text[shown.start..range.start], &text[range.end..shown.end]);
    let marked = if range.is_empty() {
        format!("{before}{CURSOR}{after}")
    } else {
        format!(
            "{before}{SELECTION_START}{}{SELECTION_END}{after}",
            &text[range.clone()]
        )
    };
    let marked = format!(
        "{}{marked}{}",
        if above > 0 {
            format!("[{above} lines above not shown]\n")
        } else {
            String::new()
        },
        if below > 0 {
            format!("\n[{below} lines below not shown]")
        } else {
            String::new()
        }
    );
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
        "{rules}Language: {}\nFile: {file}\nInstruction: {}\n{note}\n<file>\n{marked}\n</file>\n\n{REMINDER}",
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
    fn a_big_file_is_cut_to_the_lines_around_the_region() {
        let text: String = (0..5000).map(|n| format!("line {n}\n")).collect();
        let start = text.find("line 2500\n").unwrap();
        let mut big = request(start..start + "line 2500".len());
        big.text = text.clone();
        let (_, user) = build(&big);
        assert!(user.len() < MAX_FILE_BYTES + 2000, "{}", user.len());
        assert!(user.contains("<<<SELECTION>>>line 2500<<<END>>>"));
        assert!(
            user.contains("line 2499\n<<<SELECTION>>>"),
            "keeps the lines around it"
        );
        assert!(user.contains("lines above not shown]\n"));
        assert!(user.contains("lines below not shown]"));
        // Whole lines on both edges.
        let file = &user[user.find("<file>\n").unwrap() + 7..];
        let first = file.lines().nth(1).unwrap();
        assert!(first.starts_with("line "), "{first}");

        // A small file goes whole, with the format repeated at the end.
        let (_, user) = build(&request(10..19));
        assert!(!user.contains("not shown"));
        assert!(user.ends_with(REMINDER));
    }

    #[test]
    fn marks_cursor() {
        let (_, user) = build(&request(10..10));
        assert!(user.contains("fn a() {}\n<<<CURSOR>>>fn b() {}"));
        assert!(!user.contains(SELECTION_START));
    }
}
