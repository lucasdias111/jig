//! The task planner's tasks: a title, a description and a todo list, one
//! Markdown file each in the project's `.jig/tasks/`. Plain files, so they
//! read well in a diff and the agent can read them with its own tools. The
//! app shows them in the sidebar; agents create them through [`mcp`].
//!
//! ```markdown
//! # Add a task planner
//!
//! Tasks live in [`tasks::list`](crates/jig-ai/src/tasks.rs#L80).
//!
//! ## Todo
//!
//! - [x] Store tasks
//! - [ ] Show them in the sidebar
//! ```

pub mod mcp;

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, Result, bail};

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
            "{}\nTask id: {}. This step is todo {} of {}.",
            marked.to_markdown().trim_end(),
            self.id,
            todo + 1,
            self.todos.len()
        )
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

pub fn folder(project: &Path) -> PathBuf {
    project.join(FOLDER)
}

/// The file of the task `id`. Ids come from agents too, so one that could
/// reach outside the folder is refused.
pub fn path(project: &Path, id: &str) -> Result<PathBuf> {
    let fine = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if !fine {
        bail!("There's no task {id:?}.");
    }
    Ok(folder(project).join(format!("{id}.md")))
}

/// The project's tasks, newest first.
pub fn list(project: &Path) -> Result<Vec<Task>> {
    let entries = match std::fs::read_dir(folder(project)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).context("Couldn't read the tasks folder"),
    };
    let mut tasks: Vec<(SystemTime, Task)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if path.extension()? != "md" {
                return None;
            }
            let id = path.file_stem()?.to_str()?.to_string();
            let text = std::fs::read_to_string(&path).ok()?;
            let metadata = entry.metadata().ok();
            // Not every file system knows when a file was made.
            let made = metadata
                .as_ref()
                .and_then(|m| m.created().or_else(|_| m.modified()).ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some((made, Task::parse(&id, &text)))
        })
        .collect();
    tasks.sort_by(|(a, x), (b, y)| b.cmp(a).then_with(|| x.id.cmp(&y.id)));
    Ok(tasks.into_iter().map(|(_, task)| task).collect())
}

pub fn load(project: &Path, id: &str) -> Result<Task> {
    let text = std::fs::read_to_string(path(project, id)?)
        .with_context(|| format!("There's no task {id:?}."))?;
    Ok(Task::parse(id, &text))
}

/// Write `task` over its file.
pub fn save(project: &Path, task: &Task) -> Result<()> {
    let path = path(project, &task.id)?;
    std::fs::create_dir_all(folder(project)).context("Couldn't make the tasks folder")?;
    std::fs::write(&path, task.to_markdown())
        .with_context(|| format!("Couldn't save {}", path.display()))
}

/// A new task, saved under an id made from its title.
pub fn create(project: &Path, title: &str, description: &str, todos: Vec<Todo>) -> Result<Task> {
    let title = one_line(title);
    if title.is_empty() {
        bail!("A task needs a title.");
    }
    let task = Task {
        id: fresh_id(project, &title),
        title,
        description: description.trim().to_string(),
        todos,
    };
    save(project, &task)?;
    Ok(task)
}

/// `Add a task planner` → `add-a-task-planner`, with `-2` and so on when
/// that's taken.
fn fresh_id(project: &Path, title: &str) -> String {
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
    let taken = |id: &str| path(project, id).is_ok_and(|path| path.exists());
    if !taken(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken(id))
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
    fn creates_lists_and_saves_tasks() {
        let project = tempfile::tempdir().unwrap();
        assert!(list(project.path()).unwrap().is_empty(), "no folder yet");
        let first = create(
            project.path(),
            "Add a planner!",
            "Why",
            vec![Todo::new("one\ntwo")],
        )
        .unwrap();
        assert_eq!(first.id, "add-a-planner");
        assert_eq!(first.todos[0].text, "one two");
        let second = create(project.path(), "Add a planner", "", vec![]).unwrap();
        assert_eq!(second.id, "add-a-planner-2");
        let mut loaded = load(project.path(), "add-a-planner").unwrap();
        assert_eq!(loaded, first);
        loaded.todos[0].done = true;
        save(project.path(), &loaded).unwrap();
        let ids: Vec<String> = list(project.path())
            .unwrap()
            .into_iter()
            .map(|task| task.id)
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(load(project.path(), "add-a-planner").unwrap().todos[0].done);
    }

    #[test]
    fn refuses_ids_that_leave_the_folder() {
        let project = Path::new("/p");
        assert!(path(project, "../secrets").is_err());
        assert!(path(project, "a/b").is_err());
        assert!(path(project, "").is_err());
        assert!(path(project, "fine-id_2").is_ok());
        assert!(create(project, "  ", "", vec![]).is_err());
    }

    #[test]
    fn a_step_comes_with_its_whole_task() {
        let task = Task::parse("add-a-planner", FILE);
        let context = task.step_context(1);
        assert!(context.starts_with("# Add a planner\n"));
        assert!(context.contains("- [x] Store tasks\n"));
        assert!(context.contains("- [ ] Show them in the sidebar ← this step\n"));
        assert!(context.ends_with("Task id: add-a-planner. This step is todo 2 of 3."));
    }
}
