//! The task planner's tasks: a title, a description and a todo list, one
//! Markdown file each in the project's `.jig/tasks/`. Plain files, so they
//! read well in a diff and the agent can read them with its own tools. The
//! app shows them in the sidebar; agents create them through [`mcp`].
//!
//! ```markdown
//! # Add a task planner
//!
//! Tasks live in [`tasks::list`](crates/jig-ai/src/store/persist/tasks.rs#L40).
//!
//! ## Todo
//!
//! - [x] Store tasks
//! - [ ] Show them in the sidebar
//! ```

pub mod mcp;

use anyhow::{Result, bail};

/// Where a project keeps its tasks, beside `.jig/run.toml`.
pub const FOLDER: &str = ".jig/tasks";

/// The heading the todo list sits under.
const TODO_HEADING: &str = "## Todo";

/// The longest id made from a title, in characters.
const ID_CHARS: usize = 48;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Task {
    /// Its file's name without `.md`.
    pub id: String,
    pub title: String,
    /// Markdown, with code references as links.
    pub description: String,
    pub todos: Vec<Todo>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Todo {
    /// One line of Markdown.
    pub text: String,
    pub done: bool,
}

impl Todo {
    pub fn new(text: &str) -> Self {
        Self {
            text: one_line(text),
            done: false,
        }
    }
}

impl Task {
    /// Read a task file. Whatever it holds makes some task: a file without
    /// a title is titled with its id, and one without a todo heading is all
    /// description.
    pub fn parse(id: &str, text: &str) -> Self {
        let mut lines = text.lines().peekable();
        while lines.next_if(|line| line.trim().is_empty()).is_some() {}
        let title = lines
            .next_if(|line| line.starts_with("# "))
            .map(|line| line[2..].trim().to_string())
            .unwrap_or_else(|| id.to_string());
        let mut description = Vec::new();
        let mut todos: Vec<Todo> = Vec::new();
        let mut in_todos = false;
        for line in lines {
            if !in_todos {
                if is_todo_heading(line) {
                    in_todos = true;
                } else {
                    description.push(line);
                }
                continue;
            }
            if let Some(todo) = todo_item(line) {
                todos.push(todo);
            } else if !line.trim().is_empty()
                && let Some(last) = todos.last_mut()
            {
                // An item wrapped onto the next line.
                last.text.push(' ');
                last.text.push_str(line.trim());
            }
        }
        Self {
            id: id.to_string(),
            title,
            description: description.join("\n").trim().to_string(),
            todos,
        }
    }

    pub fn to_markdown(&self) -> String {
        let mut text = format!("# {}\n", one_line(&self.title));
        if !self.description.trim().is_empty() {
            text.push('\n');
            text.push_str(self.description.trim());
            text.push('\n');
        }
        if !self.todos.is_empty() {
            text.push_str(&format!("\n{TODO_HEADING}\n\n"));
            for todo in &self.todos {
                let check = if todo.done { 'x' } else { ' ' };
                text.push_str(&format!("- [{check}] {}\n", one_line(&todo.text)));
            }
        }
        text
    }

    /// The task as context for running its todo `todo`: the whole plan,
    /// that step marked, and how to check it off.
    pub fn step_context(&self, todo: usize) -> String {
        let mut marked = self.clone();
        if let Some(step) = marked.todos.get_mut(todo) {
            step.text = format!("{} ← this step", step.text);
        }
        format!(
            "{}\nTask id: {}. This step is todo {} of {}; do only this step.",
            marked.to_markdown().trim_end(),
            self.id,
            todo + 1,
            self.todos.len()
        )
    }

    /// The task as context for implementing all of it.
    pub fn context(&self) -> String {
        format!("{}\nTask id: {}.", self.to_markdown().trim_end(), self.id)
    }

    pub fn done(&self) -> usize {
        self.todos.iter().filter(|todo| todo.done).count()
    }
}

fn is_todo_heading(line: &str) -> bool {
    let Some(heading) = line.trim().strip_prefix("## ") else {
        return false;
    };
    matches!(
        heading.trim().to_lowercase().as_str(),
        "todo" | "todos" | "to do" | "to-do"
    )
}

/// `- [ ] text` or `- [x] text`, with `*` too.
fn todo_item(line: &str) -> Option<Todo> {
    let rest = line
        .trim_start()
        .strip_prefix("- ")
        .or_else(|| line.trim_start().strip_prefix("* "))?;
    let (done, text) = match rest.strip_prefix("[ ]") {
        Some(text) => (false, text),
        None => (
            true,
            rest.strip_prefix("[x]")
                .or_else(|| rest.strip_prefix("[X]"))?,
        ),
    };
    Some(Todo {
        text: text.trim().to_string(),
        done,
    })
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where tasks are kept: the project's folder, or memory in tests.
pub trait TaskStore {
    /// Every task, newest first.
    fn list(&self) -> Result<Vec<Task>>;
    fn load(&self, id: &str) -> Result<Task>;
    /// Write `task` over what's kept under its id.
    fn save(&self, task: &Task) -> Result<()>;
    /// Whether a task is kept under `id`.
    fn taken(&self, id: &str) -> bool;
}

/// Refuses an id that could reach outside the tasks folder: ids come from
/// agents too.
pub fn check_id(id: &str) -> Result<()> {
    let fine = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if !fine {
        bail!("There's no task {id:?}.");
    }
    Ok(())
}

/// A new task, saved under an id made from its title.
pub fn create(
    store: &impl TaskStore,
    title: &str,
    description: &str,
    todos: Vec<Todo>,
) -> Result<Task> {
    let title = one_line(title);
    if title.is_empty() {
        bail!("A task needs a title.");
    }
    let task = Task {
        id: fresh_id(store, &title),
        title,
        description: description.trim().to_string(),
        todos,
    };
    store.save(&task)?;
    Ok(task)
}

/// `Add a task planner` → `add-a-task-planner`, with `-2` and so on when
/// that's taken.
fn fresh_id(store: &impl TaskStore, title: &str) -> String {
    let mut base = String::new();
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            base.push(c);
        } else if !base.is_empty() && !base.ends_with('-') {
            base.push('-');
        }
        if base.chars().count() >= ID_CHARS {
            break;
        }
    }
    let base = match base.trim_end_matches('-') {
        "" => "task".to_string(),
        base => base.to_string(),
    };
    if !store.taken(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !store.taken(id))
        .unwrap_or(base)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "# Add a planner\n\nTasks live in [`list`](src/tasks.rs#L80).\n\nMore.\n\n## Todo\n\n- [x] Store tasks\n- [ ] Show them in\n  the sidebar\n- [ ] Link [`open_at`](src/a.rs)\n";

    #[test]
    fn reads_a_task_file() {
        let task = Task::parse("add-a-planner", FILE);
        assert_eq!(task.title, "Add a planner");
        assert_eq!(
            task.description,
            "Tasks live in [`list`](src/tasks.rs#L80).\n\nMore."
        );
        assert_eq!(task.todos.len(), 3);
        assert!(task.todos[0].done);
        assert_eq!(task.todos[1].text, "Show them in the sidebar");
        assert_eq!(task.done(), 1);
    }

    #[test]
    fn writes_what_it_reads() {
        let task = Task::parse("add-a-planner", FILE);
        let again = Task::parse("add-a-planner", &task.to_markdown());
        assert_eq!(task, again);
    }

    #[test]
    fn makes_do_with_any_file() {
        let task = Task::parse("notes", "Just some notes\n- [ ] not a todo yet\n");
        assert_eq!(task.title, "notes");
        assert_eq!(
            task.description, "Just some notes\n- [ ] not a todo yet",
            "no todo heading, so it's all description"
        );
        assert!(task.todos.is_empty());
        assert_eq!(Task::parse("x", "").title, "x");
    }

    #[test]
    fn a_step_comes_with_its_whole_task() {
        let task = Task::parse("add-a-planner", FILE);
        let context = task.step_context(1);
        assert!(context.starts_with("# Add a planner\n"));
        assert!(context.contains("- [x] Store tasks\n"));
        assert!(context.contains("- [ ] Show them in the sidebar ← this step\n"));
        assert!(
            context
                .ends_with("Task id: add-a-planner. This step is todo 2 of 3; do only this step.")
        );
        let whole = task.context();
        assert!(!whole.contains("← this step"));
        assert!(whole.ends_with("- [ ] Show them in the sidebar\n- [ ] Link [`open_at`](src/a.rs)\nTask id: add-a-planner."));
    }
}
