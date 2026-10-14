//! Agent mode: conversations with a full coding agent instead of a single
//! model call.

pub mod acp;
pub mod harness;
pub mod opencode;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, Result, bail};

use crate::PromptRequest;

/// Whether the agent may use a tool: after asking the user each time, on
/// its own, or not at all.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    #[default]
    Ask,
    Allow,
    Deny,
}

impl Access {
    pub const ALL: [Access; 3] = [Access::Ask, Access::Allow, Access::Deny];

    /// As `settings.toml` writes it.
    pub fn key(self) -> &'static str {
        match self {
            Access::Ask => "ask",
            Access::Allow => "allow",
            Access::Deny => "deny",
        }
    }

    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|access| access.key() == key)
            .unwrap_or_default()
    }
}

/// What the agent may do besides reading the project. Edits always wait
/// for review and nothing outside the project is touched; those aren't
/// choices.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Permissions {
    pub shell: Access,
    pub web_fetch: Access,
    pub web_search: Access,
    pub subagents: Access,
    /// Saving and updating tasks in Jig's task planner, which only writes
    /// `.jig/tasks/`. The agent asks in the conversation first, so it's
    /// allowed unless the user says otherwise.
    pub tasks: Access,
    /// Its question tool; without it, it asks in its reply.
    pub questions: bool,
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            shell: Access::Ask,
            web_fetch: Access::Ask,
            web_search: Access::Ask,
            subagents: Access::Ask,
            tasks: Access::Allow,
            questions: true,
        }
    }
}

/// The longest title Jig gives a conversation, in characters.
const TITLE_CHARS: usize = 60;

/// An earlier conversation with the agent, to pick up again.
#[derive(Clone, Debug, PartialEq)]
pub struct Conversation {
    pub id: String,
    /// What the user first asked, or `None` for conversations from before
    /// Jig named them.
    pub title: Option<String>,
    pub updated: SystemTime,
}

/// A command or skill the agent runs by name, as in "/review".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentCommand {
    pub name: String,
    /// Its first line.
    pub description: String,
    pub skill: bool,
}

pub(crate) fn canonical(directory: &Path) -> String {
    directory
        .canonicalize()
        .unwrap_or_else(|_| directory.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// A conversation's title: the start of what was first asked.
pub fn title(asked: &str) -> String {
    let line = asked.lines().next().unwrap_or_default().trim();
    if line.chars().count() <= TITLE_CHARS {
        line.to_string()
    } else {
        let cut: String = line.chars().take(TITLE_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

/// The message for the agent: the instruction, the user's note, where they
/// are in the file and what they selected. The agent finds `AGENTS.md`,
/// skills and commands on its own.
pub fn prompt(request: &PromptRequest) -> String {
    let mut out = request.instruction.trim().to_string();
    if let Some(note) = &request.comment {
        out.push_str(&format!("\nNote: {note}"));
    }
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
        let bytes = request.text.as_bytes();
        bytes[..offset.min(bytes.len())]
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count()
            + 1
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
            request.target_text().trim_end_matches('\n')
        ));
    }
    out
}

/// What the user wrote, without the note on where they are that
/// [`prompt`] adds.
pub fn asked(prompt: &str) -> &str {
    prompt.split("\n\nI'm in ").next().unwrap_or(prompt).trim()
}

/// What the agent is asked to do.
#[derive(Clone, Debug)]
pub struct AgentRequest {
    /// The project folder the agent works in.
    pub directory: PathBuf,
    /// The message, or a command's arguments.
    pub prompt: String,
    /// The model, as the agent names it: `provider/model` for OpenCode.
    pub model: String,
    /// Run this command or skill, e.g. "review", with `prompt` as its
    /// arguments.
    pub command: Option<String>,
    /// What it may do this turn.
    pub permissions: Permissions,
}

/// One edit the agent wants to make, waiting for the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EditRequest {
    pub id: String,
    pub path: PathBuf,
    pub change: EditChange,
}

/// How an agent says what an edit changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditChange {
    /// A unified diff against the file on disk. The agent writes it once
    /// it's accepted.
    Diff(String),
    /// `old` replaced by `new`, each the whole file or a part of it; `old`
    /// is `None` for a new file. The agent writes it once it's accepted.
    Replace { old: Option<String>, new: String },
    /// The file's whole new text. Jig writes it once it's accepted.
    Write(String),
}

impl EditChange {
    /// The file's text after the edit, from its text on disk.
    pub fn apply(&self, disk: &str) -> Result<String> {
        match self {
            EditChange::Diff(diff) => apply_diff(disk, diff),
            EditChange::Write(text) => Ok(text.clone()),
            EditChange::Replace { old, new } => {
                match old.as_deref().filter(|old| !old.is_empty()) {
                    None => Ok(new.clone()),
                    Some(old) if old == disk => Ok(new.clone()),
                    Some(old) => match disk.matches(old).count() {
                        1 => Ok(disk.replacen(old, new, 1)),
                        0 => bail!("the file changed since the agent read it"),
                        _ => bail!("the text to change is in the file more than once"),
                    },
                }
            }
        }
    }

    /// Jig, not the agent, puts the accepted text on disk.
    pub fn written_by_jig(&self) -> bool {
        matches!(self, EditChange::Write(_))
    }
}

/// Something else the agent wants to do that waits for the user: run a
/// command, fetch a page, start a subagent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermissionRequest {
    pub id: String,
    /// What it wants to do, e.g. "Run a command".
    pub title: String,
    /// The command, address or task.
    pub detail: String,
    /// What allowing it always would cover, as the agent remembers it,
    /// e.g. "Always allow `cargo test *`". `None` when the agent can't
    /// remember; Jig then stops asking for the rest of the conversation.
    pub always: Option<String>,
}

/// Questions the agent asks with its question tool, answered all at once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuestionRequest {
    pub id: String,
    pub questions: Vec<Question>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Question {
    pub question: String,
    /// The labels of its choices; the user may also answer in words.
    pub options: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentEvent {
    /// A short line for the UI, e.g. "Reading src/lib.rs".
    Step(String),
    Edit(EditRequest),
    Permission(PermissionRequest),
    Question(QuestionRequest),
    /// Something it did that belongs in the transcript, e.g. a command it
    /// ran: "Ran `cargo test`".
    Did(String),
    /// A subagent it started, known by the session it runs in, and what it
    /// was given to do. Sent again as more is known; the last one counts.
    Subagent {
        id: String,
        description: String,
    },
    /// A line of a subagent's own work, like [`AgentEvent::Did`].
    SubagentDid {
        id: String,
        text: String,
    },
    /// A subagent finished, with what it said last: its report back.
    SubagentDone {
        id: String,
        report: String,
    },
    /// The commands and skills it offers, when it says.
    Commands(Vec<AgentCommand>),
    /// The agent finished; its closing words.
    Done(String),
}

/// One line of an earlier conversation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Turn {
    User(String),
    Agent(String),
    Edit { path: String, accepted: bool },
    Did(String),
}

pub(crate) fn relative(path: &str, directory: &str) -> String {
    let relative = path
        .strip_prefix(directory)
        .map(|rest| rest.trim_start_matches('/'))
        .unwrap_or(path);
    if relative.is_empty() {
        ".".to_string()
    } else {
        relative.to_string()
    }
}

/// Apply a unified diff to `old`. Each hunk is looked for where its header
/// says, and anywhere later in the file if the text has moved.
pub fn apply_diff(old: &str, diff: &str) -> Result<String> {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let mut out = String::new();
    let mut cursor = 0;
    let mut lines = diff.split_inclusive('\n').peekable();
    while let Some(line) = lines.next() {
        let Some(header) = line.strip_prefix("@@ -") else {
            continue;
        };
        let start: usize = header
            .split([',', ' '])
            .next()
            .and_then(|n| n.parse().ok())
            .context("malformed hunk header")?;
        // Old and new text of the hunk, line by line.
        let (mut before, mut after): (Vec<String>, Vec<String>) = (vec![], vec![]);
        // Which side the last line went to, for "\ No newline at end of file".
        let mut last = (false, false);
        while let Some(&body) = lines.peek() {
            if body.starts_with("@@") {
                break;
            }
            lines.next();
            let (kind, text) = body.split_at(body.len().min(1));
            match kind {
                " " => {
                    before.push(text.into());
                    after.push(text.into());
                    last = (true, true);
                }
                "-" => {
                    before.push(text.into());
                    last = (true, false);
                }
                "+" => {
                    after.push(text.into());
                    last = (false, true);
                }
                "\\" => {
                    if last.0
                        && let Some(line) = before.last_mut()
                    {
                        line.truncate(line.trim_end_matches('\n').len());
                    }
                    if last.1
                        && let Some(line) = after.last_mut()
                    {
                        line.truncate(line.trim_end_matches('\n').len());
                    }
                }
                _ => {}
            }
        }
        let fits = |at: usize| {
            at + before.len() <= old_lines.len()
                && before
                    .iter()
                    .zip(&old_lines[at..])
                    .all(|(a, b)| a.trim_end_matches('\n') == b.trim_end_matches('\n'))
        };
        let expected = start.saturating_sub(1).max(cursor);
        let at = if fits(expected) {
            expected
        } else {
            (cursor..=old_lines.len())
                .find(|&at| fits(at))
                .context("the file changed since the agent read it")?
        };
        out.extend(old_lines[cursor..at].iter().copied());
        out.extend(after.iter().map(String::as_str));
        cursor = at + before.len();
    }
    out.extend(old_lines[cursor..].iter().copied());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn applies_a_unified_diff() {
        let old = "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n";
        let diff = "Index: /p/lib.rs\n===================================================================\n--- /p/lib.rs\n+++ /p/lib.rs\n@@ -1,3 +1,4 @@\n+/// Adds two integers.\n fn add(a: i32, b: i32) -> i32 {\n     a + b\n }\n";
        assert_eq!(
            apply_diff(old, diff).unwrap(),
            "/// Adds two integers.\nfn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n"
        );
    }

    #[test]
    fn applies_several_hunks_and_removals() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\n";
        let diff = "--- x\n+++ x\n@@ -1,2 +1,2 @@\n-a\n+A\n b\n@@ -7,2 +7,1 @@\n g\n-h\n";
        assert_eq!(apply_diff(old, diff).unwrap(), "A\nb\nc\nd\ne\nf\ng\n");
    }

    #[test]
    fn handles_a_missing_final_newline() {
        let old = "one\ntwo";
        let diff = "--- x\n+++ x\n@@ -1,2 +1,2 @@\n one\n-two\n\\ No newline at end of file\n+three\n\\ No newline at end of file\n";
        assert_eq!(apply_diff(old, diff).unwrap(), "one\nthree");
    }

    #[test]
    fn finds_a_hunk_that_moved() {
        let old = "x\ny\na\nb\n";
        let diff = "--- x\n+++ x\n@@ -1,2 +1,2 @@\n a\n-b\n+B\n";
        assert_eq!(apply_diff(old, diff).unwrap(), "x\ny\na\nB\n");
    }

    #[test]
    fn refuses_a_diff_that_doesnt_fit() {
        let diff = "--- x\n+++ x\n@@ -1,1 +1,1 @@\n-nope\n+yes\n";
        assert!(apply_diff("something else\n", diff).is_err());
    }

    #[test]
    fn prompt_says_where_the_user_is() {
        let text = "fn a() {}\nfn b() {}\nfn c() {}\n".to_string();
        let request = PromptRequest {
            task: None,
            instruction: "Add docs".into(),
            comment: Some("terse".into()),
            project_rules: None,
            language: "rust".into(),
            file_name: Some("src/lib.rs".into()),
            target: 10..20,
            text,
            diagnostics: Vec::new(),
        };
        assert_eq!(
            prompt(&request),
            "Add docs\nNote: terse\n\nI'm in src/lib.rs, with line 2 selected:\n```rust\nfn b() {}\n```"
        );
        let at_cursor = PromptRequest {
            task: None,
            target: 20..20,
            comment: None,
            ..request
        };
        assert!(prompt(&at_cursor).ends_with("cursor on line 3. New code goes there."));
        let with_problems = PromptRequest {
            task: None,
            diagnostics: vec![crate::prompt::Diagnostic {
                line: 3,
                severity: "error".into(),
                message: "expected `;`".into(),
            }],
            ..at_cursor
        };
        assert!(prompt(&with_problems).starts_with(
            "Add docs\nThe language server reports:\n- line 3, error: expected `;`\n\n"
        ));
        let step = PromptRequest {
            task: Some("# Plan\n\n## Todo\n\n- [ ] Add docs ← this step".into()),
            ..with_problems
        };
        let asked = prompt(&step);
        assert!(asked.starts_with("Add docs\n\nThis is from a planned task"));
        assert!(asked.contains("update_task"));
        assert!(asked.contains("<task>\n# Plan"));
    }

    #[test]
    fn replacements_apply_to_the_whole_file_or_a_part() {
        let replace = |old: Option<&str>, new: &str| EditChange::Replace {
            old: old.map(str::to_string),
            new: new.into(),
        };
        let disk = "a\nb\nc\n";
        assert_eq!(replace(Some(disk), "x\n").apply(disk).unwrap(), "x\n");
        assert_eq!(
            replace(Some("b\n"), "B\n").apply(disk).unwrap(),
            "a\nB\nc\n"
        );
        assert_eq!(replace(None, "new\n").apply("").unwrap(), "new\n");
        assert!(replace(Some("zzz"), "y").apply(disk).is_err());
        assert!(replace(Some("\n"), "y").apply(disk).is_err(), "ambiguous");
        assert!(EditChange::Write("w".into()).written_by_jig());
    }

    #[test]
    fn titles_are_the_start_of_what_was_asked() {
        assert_eq!(title("Add docs\nmore"), "Add docs");
        let long = "word ".repeat(30);
        assert_eq!(title(&long).chars().count(), TITLE_CHARS);
        assert!(title(&long).ends_with('…'));
    }
}
