//! Tasks on disk: one Markdown file each in the project's `.jig/tasks/`.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use anyhow::{Context as _, Result};

use crate::domain::tasks::{self, FOLDER, Task, TaskStore, Todo, mcp};

/// A project's tasks folder, as a [`TaskStore`].
pub struct TaskFolder<'a> {
    pub project: &'a Path,
}

impl TaskStore for TaskFolder<'_> {
    fn list(&self) -> Result<Vec<Task>> {
        list(self.project)
    }

    fn load(&self, id: &str) -> Result<Task> {
        load(self.project, id)
    }

    fn save(&self, task: &Task) -> Result<()> {
        save(self.project, task)
    }

    fn taken(&self, id: &str) -> bool {
        path(self.project, id).is_ok_and(|path| path.exists())
    }
}

pub fn folder(project: &Path) -> PathBuf {
    project.join(FOLDER)
}

/// The file of the task `id`. Ids come from agents too, so one that could
/// reach outside the folder is refused.
pub fn path(project: &Path, id: &str) -> Result<PathBuf> {
    tasks::check_id(id)?;
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
    tasks::create(&TaskFolder { project }, title, description, todos)
}

/// Answer the task tool's requests from `input` on `output`, on the
/// project's tasks, until the agent hangs up.
pub fn serve(project: &Path, input: impl BufRead, output: impl Write) -> Result<()> {
    mcp::serve(&TaskFolder { project }, input, output)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
