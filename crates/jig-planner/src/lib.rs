//! The task planner: the project's tasks, each a title, a description and
//! a todo list, from `.jig/tasks/`. Code in a task is linked; clicking a
//! link opens the code, and Go Back returns. Agents add tasks with their
//! tool ([`jig_ai::tasks::mcp`]); the workspace watches the folder, so
//! theirs show up as they're made.
//!
//! This file is the crate's facade: it only says what the rest of Jig may
//! use. The work is in its layers (`AGENTS.md`, Layers):
//!
//! - [`ui`]: the task list and the task form, as GPUI views.
//! - [`domain`]: links to code, `@` mentions, finding declarations and
//!   changing tasks, in plain Rust.
//! - [`store`]: what the window shows, and the tasks and files on disk.

mod domain;
mod store;
mod ui;

// The planner's contract: what the rest of Jig uses of it (`architecture.toml`).
pub use domain::links::{
    CodeLink, Target as LinkTarget, first_code_link, link_files, locate as locate_link,
    target as link_target,
};
pub use ui::{FORM_WIDTH, TASKS_ICON, TaskForm, TaskFormEvent, TaskPlanner, TaskPlannerEvent};
