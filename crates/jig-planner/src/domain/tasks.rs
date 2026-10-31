//! Working on the project's tasks: changing one as it is now, and saving
//! what the form holds. Where the tasks are kept is the store's
//! ([`TaskStore`]); this only says what to do with them.

use anyhow::Result;
use jig_ai::tasks::{Task, Todo};

/// The project's tasks, wherever they're kept. The store keeps them in
/// `.jig/tasks/`; tests keep them in memory.
pub trait TaskStore {
    /// Every task, newest first.
    fn list(&self) -> Result<Vec<Task>>;
    fn load(&self, id: &str) -> Result<Task>;
    /// Write `task` over the one with its id.
    fn save(&self, task: &Task) -> Result<()>;
    /// A new task, kept under an id made from its title.
    fn create(&self, title: &str, description: &str, todos: Vec<Todo>) -> Result<Task>;
    /// Put task `id` where it can be got back from, as the Trash.
    fn discard(&self, id: &str) -> Result<()>;
}

/// Change task `id` as it is now, which an agent may have changed since it
/// was shown, and save it.
pub fn change(store: &impl TaskStore, id: &str, change: impl FnOnce(&mut Task)) -> Result<()> {
    let mut task = store.load(id)?;
    change(&mut task);
    store.save(&task)
}

/// Check todo `ix` of task `id` off, or back on.
pub fn toggle_todo(store: &impl TaskStore, id: &str, ix: usize) -> Result<()> {
    change(store, id, |task| {
        if let Some(todo) = task.todos.get_mut(ix) {
            todo.done = !todo.done;
        }
    })
}

/// Save what the form holds: over task `id`, or as a new task without one.
/// Returns the task's id.
pub fn save_form(
    store: &impl TaskStore,
    id: Option<String>,
    title: String,
    description: String,
    todos: Vec<Todo>,
) -> Result<String> {
    match id {
        Some(id) => {
            let task = Task {
                id: id.clone(),
                title,
                description,
                todos,
            };
            store.save(&task).map(|()| id)
        }
        None => store
            .create(&title, &description, todos)
            .map(|task| task.id),
    }
}

/// Todo `todo` of task `id` as it is now, to hand to the agent; `None` when
/// the todo has gone since it was shown.
pub fn delegated(store: &impl TaskStore, id: &str, todo: usize) -> Result<Option<Task>> {
    let task = store.load(id)?;
    Ok((todo < task.todos.len()).then_some(task))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use anyhow::anyhow;

    use super::*;

    /// Tasks in memory, by id.
    #[derive(Default)]
    struct Tasks(RefCell<Vec<Task>>);

    impl TaskStore for Tasks {
        fn list(&self) -> Result<Vec<Task>> {
            Ok(self.0.borrow().clone())
        }

        fn load(&self, id: &str) -> Result<Task> {
            let tasks = self.0.borrow();
            let task = tasks.iter().find(|task| task.id == id);
            task.cloned()
                .ok_or_else(|| anyhow!("There's no task {id:?}."))
        }

        fn save(&self, task: &Task) -> Result<()> {
            let mut tasks = self.0.borrow_mut();
            tasks.retain(|t| t.id != task.id);
            tasks.push(task.clone());
            Ok(())
        }

        fn create(&self, title: &str, description: &str, todos: Vec<Todo>) -> Result<Task> {
            let task = Task {
                id: title.to_lowercase().replace(' ', "-"),
                title: title.into(),
                description: description.into(),
                todos,
            };
            self.save(&task)?;
            Ok(task)
        }

        fn discard(&self, id: &str) -> Result<()> {
            self.0.borrow_mut().retain(|task| task.id != id);
            Ok(())
        }
    }

    #[test]
    fn saves_a_new_task_then_changes_it() {
        let store = Tasks::default();
        let id = save_form(
            &store,
            None,
            "Add a planner".into(),
            String::new(),
            vec![Todo::new("Read the tasks")],
        )
        .unwrap();
        assert_eq!(id, "add-a-planner");
        toggle_todo(&store, &id, 0).unwrap();
        assert!(store.load(&id).unwrap().todos[0].done);
        toggle_todo(&store, &id, 5).unwrap();
        assert_eq!(
            store.list().unwrap().len(),
            1,
            "a todo that's gone changes nothing"
        );

        let id = save_form(&store, Some(id), "Renamed".into(), "Why".into(), vec![]).unwrap();
        let task = store.load(&id).unwrap();
        assert_eq!((task.title.as_str(), task.todos.len()), ("Renamed", 0));
    }

    #[test]
    fn delegates_only_todos_still_there() {
        let store = Tasks::default();
        let id = save_form(
            &store,
            None,
            "Plan".into(),
            String::new(),
            vec![Todo::new("One")],
        )
        .unwrap();
        assert!(delegated(&store, &id, 0).unwrap().is_some());
        assert!(delegated(&store, &id, 1).unwrap().is_none());
        store.discard(&id).unwrap();
        assert!(delegated(&store, &id, 0).is_err());
    }
}
