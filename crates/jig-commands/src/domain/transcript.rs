//! The conversation's transcript as data: its entries, what it's waiting
//! on, and how its rows group, in plain Rust. The views in `ui::chat` draw
//! it.

use std::sync::{Arc, Mutex};
use std::time::Instant;

/// One line of the conversation.
#[derive(Clone, Debug, PartialEq)]
pub enum ChatEntry {
    /// What the user asked or replied.
    User(String),
    /// The agent's closing words for a turn.
    Agent(String),
    /// An edit the agent proposed, and whether the user took it. `diff`
    /// is what it changed, opened from its row; empty when not known, as
    /// for an earlier conversation picked up again.
    Edit {
        path: String,
        accepted: bool,
        diff: Vec<DiffLine>,
    },
    /// Something the agent did besides editing, e.g. "Ran `cargo test`".
    Did(String),
    /// A subagent it started: what it was given to do, each thing it did,
    /// and once it's done, its report back. Opened from its row.
    Subagent {
        id: String,
        description: String,
        calls: Vec<String>,
        report: Option<String>,
    },
    /// Something the agent drew on the Map: its summary, whether it shows
    /// there and whether it's pinned. `stepped` drawings can be toured;
    /// `settled` says what became of a proposal ("Kept", "Dismissed").
    Drawing {
        id: usize,
        summary: String,
        shown: bool,
        pinned: bool,
        stepped: bool,
        settled: Option<String>,
    },
    /// Something that happened to the run, e.g. "Stopped."
    Note(String),
    Error(String),
}

/// What a drawing's row in the transcript was asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrawingAction {
    /// Show it on the Map, or hide it.
    Toggle,
    Pin,
    Tour,
}

/// One line of an edit's diff.
#[derive(Clone, Debug, PartialEq)]
pub enum DiffLine {
    /// Unchanged, shown around a change.
    Same(String),
    Removed(String),
    Added(String),
    /// Unchanged lines left out between two changes.
    Gap,
}

/// What the conversation is waiting on.
#[derive(Clone, Debug, PartialEq)]
pub enum ChatStatus {
    /// The agent is on a turn.
    Working { started: Instant, step: LiveStep },
    /// An edit is in the buffer awaiting accept or reject.
    Reviewing { path: String, removed: String },
    /// The agent wants to do something else that needs the user's go-ahead,
    /// e.g. run a command.
    Asking {
        title: String,
        detail: String,
        /// The label of the button that allows it always.
        always: String,
    },
    /// The agent proposes saving its plan as a task: the user saves it,
    /// turns it down, or says in the reply box what to change.
    SaveTask { title: String, todos: Vec<String> },
    /// The agent asked a question; the user picks an option or answers in
    /// the reply box.
    Question {
        question: String,
        options: Vec<String>,
        /// Which of how many questions this is, when it asked several.
        position: (usize, usize),
    },
    /// The agent wants the user to sign in; each option is a way to.
    SignIn {
        options: Vec<String>,
        /// A terminal was opened for the login.
        opened: bool,
    },
    /// An earlier conversation is being fetched.
    Loading,
    /// The turn is over; the user may reply.
    Waiting,
}

/// The conversation's model, shown in its header and picked from there.
#[derive(Clone, Debug, Default)]
pub struct ChatModels {
    /// Each model's label, e.g. "Opus 5.5".
    pub options: Vec<String>,
    pub current: Option<usize>,
    /// The list is open.
    pub open: bool,
}

/// Text shared with a background request, read on every frame.
#[derive(Clone, Default)]
pub struct LiveStep(Arc<Mutex<String>>);

impl LiveStep {
    pub fn set(&self, text: String) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = text;
    }

    pub fn get(&self) -> String {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl PartialEq for LiveStep {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl std::fmt::Debug for LiveStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("LiveStep").field(&self.get()).finish()
    }
}

/// A row of the transcript as drawn: one entry, or a run of two or more
/// tool calls folded into one "Used 5 tools" row, so the agent's words
/// aren't lost among its `grep`s. Each carries its first entry's index.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Entry(usize, ChatEntry),
    Calls(usize, Vec<String>),
}

/// The transcript's rows, tool calls in a row folded together.
pub fn rows(entries: Vec<ChatEntry>) -> Vec<Row> {
    let mut rows = Vec::new();
    let mut entries = entries.into_iter().enumerate().peekable();
    while let Some((index, entry)) = entries.next() {
        let ChatEntry::Did(first) = entry else {
            rows.push(Row::Entry(index, entry));
            continue;
        };
        let mut calls = vec![first];
        while let Some((_, ChatEntry::Did(_))) = entries.peek() {
            if let Some((_, ChatEntry::Did(text))) = entries.next() {
                calls.push(text);
            }
        }
        rows.push(if calls.len() == 1 {
            Row::Entry(index, ChatEntry::Did(calls.remove(0)))
        } else {
            Row::Calls(index, calls)
        });
    }
    rows
}

/// How many lines an edit's diff adds and removes.
pub fn diff_counts(diff: &[DiffLine]) -> (usize, usize) {
    let count = |kind: fn(&DiffLine) -> bool| diff.iter().filter(|line| kind(line)).count();
    let added = count(|line| matches!(line, DiffLine::Added(_)));
    let removed = count(|line| matches!(line, DiffLine::Removed(_)));
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_calls_in_a_row_fold_into_one() {
        let rows = rows(vec![
            ChatEntry::User("look".into()),
            ChatEntry::Did("Read a.rs".into()),
            ChatEntry::Did("Read b.rs".into()),
            ChatEntry::Agent("Done.".into()),
            ChatEntry::Did("Ran ls".into()),
        ]);
        assert_eq!(
            rows,
            [
                Row::Entry(0, ChatEntry::User("look".into())),
                Row::Calls(1, vec!["Read a.rs".into(), "Read b.rs".into()]),
                Row::Entry(3, ChatEntry::Agent("Done.".into())),
                Row::Entry(4, ChatEntry::Did("Ran ls".into())),
            ]
        );
    }

    #[test]
    fn diff_counts_added_and_removed_lines() {
        let diff = [
            DiffLine::Same("a".into()),
            DiffLine::Removed("b".into()),
            DiffLine::Added("c".into()),
            DiffLine::Added("d".into()),
            DiffLine::Gap,
        ];
        assert_eq!(diff_counts(&diff), (2, 1));
    }
}
