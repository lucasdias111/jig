//! The disk: the project's tasks, one Markdown file each in `.jig/tasks/`
//! (read and written by [`jig_ai::tasks`], which the agents' task tool
//! shares), and its files for the `@` list.

use std::path::{Path, PathBuf};

use anyhow::Result;
use jig_ai::tasks::{self, Task, Todo};

use crate::domain::symbols::ProjectFiles;
use crate::domain::tasks::TaskStore;

/// The tasks in a project's `.jig/tasks/`.
pub struct TaskFolder {
    project: PathBuf,
}

impl TaskFolder {
    pub fn new(project: &Path) -> Self {
        Self {
            project: project.to_path_buf(),
        }
    }
}

impl TaskStore for TaskFolder {
    fn list(&self) -> Result<Vec<Task>> {
        tasks::list(&self.project)
    }

    fn load(&self, id: &str) -> Result<Task> {
        tasks::load(&self.project, id)
    }

    fn save(&self, task: &Task) -> Result<()> {
        tasks::save(&self.project, task)
    }

    fn create(&self, title: &str, description: &str, todos: Vec<Todo>) -> Result<Task> {
        tasks::create(&self.project, title, description, todos)
    }

    /// Moves its file to the Trash, so deleting it can be undone.
    fn discard(&self, id: &str) -> Result<()> {
        let path = tasks::path(&self.project, id)?;
        trash::delete(path)?;
        Ok(())
    }
}

/// A project's files on disk, as `.gitignore` leaves them.
pub struct ProjectFolder {
    root: PathBuf,
}

impl ProjectFolder {
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }
}

impl ProjectFiles for ProjectFolder {
    fn files(&self) -> Vec<String> {
        jig_commands::project_files::walk(&self.root)
    }

    fn size(&self, path: &str) -> Option<u64> {
        std::fs::metadata(self.root.join(path))
            .ok()
            .map(|metadata| metadata.len())
    }

    fn read(&self, path: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(path)).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::symbols;

    #[test]
    fn indexes_a_projects_code_and_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/a.rs"), "fn run() {}\n").unwrap();
        std::fs::write(dir.path().join("README.md"), "type the name\n").unwrap();
        std::fs::create_dir_all(dir.path().join(".jig/tasks")).unwrap();
        std::fs::write(dir.path().join(".jig/tasks/a.md"), "# A\n").unwrap();
        let index = symbols::index(&ProjectFolder::new(dir.path()));
        let names: Vec<&str> = index.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["run"]);
        assert_eq!(index.files, ["README.md", "src/a.rs"]);
    }

    #[test]
    fn keeps_tasks_in_the_project() {
        let dir = tempfile::tempdir().unwrap();
        let folder = TaskFolder::new(dir.path());
        let task = folder.create("Add a planner", "", Vec::new()).unwrap();
        assert!(dir.path().join(".jig/tasks/add-a-planner.md").exists());
        assert_eq!(folder.load(&task.id).unwrap().title, "Add a planner");
        assert_eq!(folder.list().unwrap().len(), 1);
    }
}
