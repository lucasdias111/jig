//! The request sent to the model.

use std::ops::Range;

pub const SELECTION_START: &str = "<<<SELECTION>>>";
pub const SELECTION_END: &str = "<<<END>>>";
pub const CURSOR: &str = "<<<CURSOR>>>";

/// The most words the message beside an edit may have.
pub const MESSAGE_WORDS: usize = 20;
/// The most words an answer may have: room for what the code is for and
/// the detail that's easy to misread, which 20 words rarely fit.
pub const ANSWER_WORDS: usize = 80;

const SYSTEM: &str = r#"You are Jig, a precise code-editing tool inside a text editor. You are not a chat assistant.

You receive a file in which the region to change is marked:
- <<<SELECTION>>> ... <<<END>>> surrounds code to replace, or
- <<<CURSOR>>> marks where new code is inserted.

Apply the instruction to that region only. A Note, when present, is the user's detail for this run (a name, a constraint, a choice); follow it. Diagnostics, when present, are the problems a language server reports on the region's lines, by line number in the file; they are what to fix. Project rules, when present, are the project's conventions; follow them unless the instruction or Note says otherwise. A task, when present, is the plan the instruction is one step of; it's context, so do only that step, in the marked region. Reply with exactly one JSON object and nothing else:
{"replace": "<new text for the region>", "message": "<one short sentence>"}

Rules for "replace":
- It is the complete new text for the marked region (or the text to insert at the cursor). It replaces the region verbatim.
- Never include the markers or any code outside the region.
- Match the file's existing indentation and style. No markdown fences.
- If the instruction is a question or asks for an explanation, return the region unchanged and answer in "message".

Rules for "message": plain text, no greetings, no follow-up questions. For an edit, at most 20 words saying what you changed, and only what you actually changed in "replace". For an answer, at most 80 words."#;

/// For jigs that answer about the code, such as Explain: nothing comes
/// back to put in the buffer, so the model can't change the code by
/// retyping it, and the answer has room to be specific.
const ANSWER_SYSTEM: &str = r#"You are Jig, a precise code-reading tool inside a text editor. You are not a chat assistant.

You receive a file in which the region the user asks about is marked:
- <<<SELECTION>>> ... <<<END>>> surrounds the code in question, or
- <<<CURSOR>>> marks the place in question.

Answer the instruction about that region only; the rest of the file is context. A Note, when present, is the user's detail for this run; follow it. Diagnostics, when present, are the problems a language server reports on the region's lines, by line number in the file. Project rules, when present, are the project's conventions. A task, when present, is the plan the instruction is one step of; it's context. Reply with exactly one JSON object and nothing else:
{"message": "<the answer>"}

Rules for "message":
- Two or three short sentences, at most 80 words, plain text, no markdown, no greetings, no follow-up questions.
- Be specific to this code: name what it does and the detail that is easiest to misread (an edge case, a tie-break, an order, what it returns when nothing matches).
- Say only what the code shows. If something depends on code you can't see, say so rather than guess.
- Don't restate the instruction or describe your reply."#;

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
    /// What the language server reports on the target's lines, for jigs
    /// that fix problems.
    pub diagnostics: Vec<Diagnostic>,
    /// The planned task the instruction is one step of, when a todo from
    /// the task planner is run: the whole task, that step marked.
    pub task: Option<String>,
    /// Ask for an answer about the region rather than its new text, for
    /// jigs like Explain. The buffer is never changed.
    pub answer: bool,
}

/// One problem a language server reports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// 1-based, in the whole file.
    pub line: usize,
    /// `error`, `warning`, `info` or `hint`.
    pub severity: String,
    pub message: String,
}

impl Diagnostic {
    /// `line 3, error: mismatched types`, the way prompts list it.
    pub fn describe(&self) -> String {
        // Messages can run over several lines; one line each keeps the
        // list readable.
        let message = self
            .message
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        format!("line {}, {}: {message}", self.line, self.severity)
    }
}

impl PromptRequest {
    /// The text the command replaces; empty when [`Self::check_target`]
    /// would fail, so a bad range can't panic here.
    pub fn target_text(&self) -> &str {
        self.text.get(self.target.clone()).unwrap_or_default()
    }

    pub fn check_target(&self) -> anyhow::Result<()> {
        let Range { start, end } = self.target;
        if start > end || self.text.get(start..end).is_none() {
            anyhow::bail!(
                "The region to edit ({start}..{end}) isn't within the file's {} bytes.",
                self.text.len()
            );
        }
        Ok(())
    }
}

/// Files longer than this are sent as a window around the region: a model
/// reading a huge file is slower and more likely to forget the reply format.
const MAX_FILE_BYTES: usize = 24 * 1024;

/// Repeated after the file, where the model reads it last.
const REMINDER: &str = "Reply with the JSON object only: {\"replace\": ..., \"message\": ...}. No prose before or after it.";
const ANSWER_REMINDER: &str =
    "Reply with the JSON object only: {\"message\": ...}. No prose before or after it.";

/// The part of `text` to send for `range`: all of it when it's small,
/// otherwise whole lines around `range`, about [`MAX_FILE_BYTES`] in all.
fn window(text: &str, range: &Range<usize>) -> Range<usize> {
    if text.len() <= MAX_FILE_BYTES {
        return 0..text.len();
    }
    let spare = MAX_FILE_BYTES.saturating_sub(range.len());
    let start = range.start.saturating_sub(spare / 2);
    let end = (range.end + spare - (range.start - start)).min(text.len());
    // Whole lines only. `start` and `end` may fall inside a multibyte
    // character, so look for newlines in the bytes: the byte after one is
    // always a character boundary.
    let bytes = text.as_bytes();
    let start = bytes[..start]
        .iter()
        .rposition(|&byte| byte == b'\n')
        .map_or(0, |newline| newline + 1);
    let start = start.min(range.start);
    let end = bytes[end..]
        .iter()
        .position(|&byte| byte == b'\n')
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
    let diagnostics = if request.diagnostics.is_empty() {
        String::new()
    } else {
        let lines: Vec<String> = request
            .diagnostics
            .iter()
            .map(Diagnostic::describe)
            .collect();
        format!("<diagnostics>\n{}\n</diagnostics>\n", lines.join("\n"))
    };
    let rules = request
        .project_rules
        .as_deref()
        .map(|rules| format!("<project_rules>\n{}\n</project_rules>\n\n", rules.trim()))
        .unwrap_or_default();
    let task = request
        .task
        .as_deref()
        .map(|task| format!("<task>\n{}\n</task>\n\n", task.trim()))
        .unwrap_or_default();
    let (system, reminder) = if request.answer {
        (ANSWER_SYSTEM, ANSWER_REMINDER)
    } else {
        (SYSTEM, REMINDER)
    };
    let user = format!(
        "{rules}{task}Language: {}\nFile: {file}\nInstruction: {}\n{note}{diagnostics}\n<file>\n{marked}\n</file>\n\n{reminder}",
        request.language, request.instruction
    );
    (system.to_string(), user)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(target: Range<usize>) -> PromptRequest {
        PromptRequest {
            task: None,
            instruction: "Add docs".into(),
            language: "rust".into(),
            file_name: Some("lib.rs".into()),
            text: "fn a() {}\nfn b() {}\n".into(),
            target,
            comment: None,
            project_rules: None,
            diagnostics: Vec::new(),
            answer: false,
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
    fn includes_the_task_a_step_is_from() {
        let mut step = request(10..19);
        step.task = Some("# Plan\n- [ ] Add docs ← this step".into());
        let (system, user) = build(&step);
        assert!(system.contains("do only that step"));
        assert!(
            user.starts_with("<task>\n# Plan\n- [ ] Add docs ← this step\n</task>\n\nLanguage")
        );
        assert!(!build(&request(10..19)).1.contains("<task>"));
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
    fn includes_diagnostics_after_the_note() {
        let mut with_problems = request(10..19);
        with_problems.comment = Some("terse".into());
        with_problems.diagnostics = vec![
            Diagnostic {
                line: 2,
                severity: "error".into(),
                message: "mismatched types\nexpected `u32`".into(),
            },
            Diagnostic {
                line: 2,
                severity: "warning".into(),
                message: "unused variable `x`".into(),
            },
        ];
        let (system, user) = build(&with_problems);
        assert!(system.contains("Diagnostics, when present"));
        assert!(user.contains(
            "Note: terse\n<diagnostics>\nline 2, error: mismatched types expected `u32`\nline 2, warning: unused variable `x`\n</diagnostics>\n\n<file>\n"
        ));
        let (_, user) = build(&request(10..19));
        assert!(!user.contains("<diagnostics>"));
        assert!(user.contains("Instruction: Add docs\n\n<file>\n"));
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
    fn a_big_multibyte_file_is_cut_without_splitting_a_character() {
        // Lines of three-byte characters, so the window's first guess at
        // its edges lands inside one wherever the region is.
        let text: String = (0..3000).map(|n| format!("é{n}→€€€€\n")).collect();
        for offset in [0, 1, 7, 100, 1000] {
            let line = format!("é{}→", 1500 + offset);
            let start = text.find(&line).unwrap();
            let mut big = request(start..start + line.len());
            big.text = text.clone();
            big.check_target().unwrap();
            let (_, user) = build(&big);
            assert!(user.contains(&format!("<<<SELECTION>>>{line}<<<END>>>")));
            assert!(user.contains("lines above not shown]\n"));
        }
    }

    #[test]
    fn a_target_outside_the_text_or_inside_a_character_is_refused() {
        let mut bad = request(0..100);
        assert!(bad.check_target().is_err());
        assert_eq!(bad.target_text(), "");
        bad.text = "é".into();
        bad.target = 1..2;
        assert!(bad.check_target().is_err());
        // A reversed range, as a stale offset could make.
        bad.target = Range { start: 2, end: 1 };
        assert!(bad.check_target().is_err());
        bad.target = 0..2;
        bad.check_target().unwrap();
        assert_eq!(bad.target_text(), "é");
    }

    #[test]
    fn an_answer_asks_for_the_message_alone() {
        let mut question = request(10..19);
        question.answer = true;
        let (system, user) = build(&question);
        assert!(system.contains("{\"message\": \"<the answer>\"}"));
        assert!(!system.contains("\"replace\""));
        assert!(system.contains("at most 80 words"));
        assert!(user.contains("<<<SELECTION>>>fn b() {}<<<END>>>"));
        assert!(user.ends_with(ANSWER_REMINDER));
        // An edit still asks for the new text.
        assert!(build(&request(10..19)).0.contains("\"replace\""));
    }

    #[test]
    fn marks_cursor() {
        let (_, user) = build(&request(10..10));
        assert!(user.contains("fn a() {}\n<<<CURSOR>>>fn b() {}"));
        assert!(!user.contains(SELECTION_START));
    }
}
