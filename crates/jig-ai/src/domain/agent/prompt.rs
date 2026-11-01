//! The words a conversation starts from and the messages that follow:
//! what the user asked, where they are, and the code they sent.

use std::ops::Range;

/// What a conversation starts from: what the user asked, and where they
/// are in the file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prompt {
    pub instruction: String,
    /// What the language server reports on the target's lines, for fixes.
    pub diagnostics: Vec<Diagnostic>,
    /// The planned task the instruction is one step of, when a todo from
    /// the task planner is run: the whole task, that step marked.
    pub task: Option<String>,
    pub language: String,
    /// Relative to the project.
    pub file_name: Option<String>,
    pub text: String,
    /// The selection, or the cursor when empty.
    pub target: Range<usize>,
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

/// The message for the agent: the instruction, where the user is in the
/// file and what they selected. The agent finds `AGENTS.md`, skills and
/// commands on its own.
pub fn prompt(request: &Prompt) -> String {
    let mut out = request.instruction.trim().to_string();
    if let Some(task) = &request.task {
        out.push_str(&format!(
            "\n\nThis is from a planned task in Jig's task planner, below. As you finish each of its todos, check it off with the task tool's update_task.\n\n<task>\n{}\n</task>",
            task.trim()
        ));
    }
    if !request.diagnostics.is_empty() {
        out.push_str("\nThe language server reports:");
        for diagnostic in &request.diagnostics {
            out.push_str(&format!("\n- {}", diagnostic.describe()));
        }
    }
    let file = request.file_name.as_deref().unwrap_or("an unsaved file");
    let line = |offset: usize| {
        1 + request
            .text
            .bytes()
            .take(offset)
            .filter(|&b| b == b'\n')
            .count()
    };
    let range = request.target.clone();
    out.push_str("\n\n");
    if range.is_empty() {
        out.push_str(&format!(
            "I'm in {file}, with the cursor on line {}. New code goes there.",
            line(range.start)
        ));
    } else if range == (0..request.text.len()) {
        out.push_str(&format!("I'm in {file}; this is about the whole file."));
    } else {
        let (first, last) = (line(range.start), line(range.end.saturating_sub(1)));
        let lines = if first == last {
            format!("line {first}")
        } else {
            format!("lines {first}-{last}")
        };
        out.push_str(&format!(
            "I'm in {file}, with {lines} selected:\n```{}\n{}\n```",
            request.language,
            request
                .text
                .get(range.clone())
                .unwrap_or_default()
                .trim_end_matches('\n')
        ));
    }
    out.push_str(LINK_THE_CODE);
    out
}

/// Asked of every conversation, so the code the agent mentions opens with
/// a click in Jig's transcript.
pub(super) const LINK_THE_CODE: &str = "\n\nWhen you mention code in your reply, link it, as [`name`](path#L12) with the path from the project's folder.";

/// Code the user sent the agent from the editor, to write about in their
/// next message: named in it as `@src/a.rs:10-12`, sent with its text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeReference {
    /// Relative to the project.
    pub file: String,
    pub language: String,
    /// 1-based, first and last.
    pub lines: (usize, usize),
    pub code: String,
}

impl CodeReference {
    /// `src/a.rs:10-12`, or `src/a.rs:10` for one line.
    pub fn name(&self) -> String {
        match self.lines {
            (first, last) if first == last => format!("{}:{first}", self.file),
            (first, last) => format!("{}:{first}-{last}", self.file),
        }
    }
}

/// Starts the code a message refers to, so reading the conversation back
/// shows only what the user wrote.
const REFERRED: &str = "\n\nThe code I refer to:";

/// A message the user wrote in the conversation, with the code it refers
/// to. The `first` of a conversation is asked to link code too, as
/// [`prompt`]'s are.
pub fn message(text: &str, references: &[CodeReference], first: bool) -> String {
    let mut out = text.trim().to_string();
    if !references.is_empty() {
        out.push_str(REFERRED);
        for reference in references {
            out.push_str(&format!(
                "\n\n@{}:\n```{}\n{}\n```",
                reference.name(),
                reference.language,
                reference.code.trim_end_matches('\n')
            ));
        }
    }
    if first {
        out.push_str(LINK_THE_CODE);
    }
    out
}

/// What the user wrote, without the note on where they are that
/// [`prompt`] adds or the code [`message`] sends with it.
pub fn asked(prompt: &str) -> &str {
    ["\n\nI'm in ", REFERRED, LINK_THE_CODE]
        .iter()
        .fold(prompt, |text, marker| {
            text.split(marker).next().unwrap_or(text)
        })
        .trim()
}
