//! What this window's planner shows: the tasks as last read, which are
//! folded open, and which it has seen, so new ones (an agent's, say) open
//! as they arrive. Kept in memory only.

use std::collections::HashSet;

use jig_ai::tasks::Task;

#[derive(Default)]
pub struct Board {
    tasks: Vec<Task>,
    /// Tasks folded open, by id.
    expanded: HashSet<String>,
    /// Every task seen, to open the ones that arrive.
    known: Option<HashSet<String>>,
}

impl Board {
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    pub fn is_expanded(&self, id: &str) -> bool {
        self.expanded.contains(id)
    }

    /// Take the tasks as just read, or keep the last ones when reading
    /// failed. Tasks that weren't there before are opened, and `true`
    /// returned; the first read opens none.
    pub fn refresh(&mut self, read: Option<Vec<Task>>) -> bool {
        if let Some(tasks) = read {
            self.tasks = tasks;
        }
        let present: HashSet<String> = self.tasks.iter().map(|task| task.id.clone()).collect();
        self.expanded.retain(|id| present.contains(id));
        let arrived: Vec<String> = match &self.known {
            Some(known) => present.difference(known).cloned().collect(),
            None => Vec::new(),
        };
        self.expanded.extend(arrived.iter().cloned());
        self.known = Some(present);
        !arrived.is_empty()
    }

    pub fn toggle(&mut self, id: &str) {
        if !self.expanded.remove(id) {
            self.expanded.insert(id.to_string());
        }
    }

    pub fn expand(&mut self, id: &str) {
        self.expanded.insert(id.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str) -> Task {
        Task {
            id: id.into(),
            ..Task::default()
        }
    }

    #[test]
    fn opens_tasks_that_arrive_after_the_first_read() {
        let mut board = Board::default();
        assert!(
            !board.refresh(Some(vec![task("a")])),
            "the first read opens none"
        );
        assert!(!board.is_expanded("a"));
        assert!(board.refresh(Some(vec![task("b"), task("a")])));
        assert!(board.is_expanded("b"));
        board.toggle("b");
        assert!(!board.refresh(None), "a failed read keeps what was there");
        assert_eq!(board.tasks().len(), 2);
        board.expand("a");
        board.refresh(Some(vec![task("b")]));
        assert!(!board.is_expanded("a"), "gone");
    }
}
