//! Planner mode (⌘2 or ⌃⇧T): the project's tasks, each a title, a
//! description and a todo list. Click a task to fold it open, check its
//! todos off, add one, or edit and delete it. Clicking a link in one opens
//! the code. Drawing it is [`view`]'s.

mod view;

use std::path::{Path, PathBuf};

use gpui_kit::component::input::{Escape, InputEvent, InputState};
use gpui_kit::*;
use jig_ai::tasks::{Task, Todo};

use crate::domain::links::{self, CodeLink, Target};
use crate::domain::tasks;
use crate::domain::tasks::TaskStore as _;
use crate::store::persist::TaskFolder;
use crate::store::session::Board;

pub enum TaskPlannerEvent {
    /// Open the code a link points at, in the file at this full path.
    Open {
        path: PathBuf,
        link: CodeLink,
    },
    /// Open the form for this task, or `None` for a new one.
    Edit(Option<String>),
    /// Hand todo `todo` of `task` to the agent, the whole task coming along
    /// as context.
    Delegate {
        task: Task,
        todo: usize,
    },
    /// Hand all of `task` to the agent, to work through its todos.
    Implement(Task),
    Dismissed,
}

/// The field under an open task for adding a todo to it.
struct QuickAdd {
    id: String,
    input: Entity<InputState>,
    _subscription: Subscription,
}

pub struct TaskPlanner {
    project: PathBuf,
    store: TaskFolder,
    board: Board,
    quick_add: Option<QuickAdd>,
    error: Option<String>,
    focus: FocusHandle,
    scroll: ScrollHandle,
}

impl EventEmitter<TaskPlannerEvent> for TaskPlanner {}

impl Focusable for TaskPlanner {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TaskPlanner {
    pub fn new(project: &Path, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            project: project.to_path_buf(),
            store: TaskFolder::new(project),
            board: Board::default(),
            quick_add: None,
            error: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        };
        this.reload(cx);
        this
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn tasks(&self) -> &[Task] {
        self.board.tasks()
    }

    /// Whether the panel, or a field in it, has the keyboard.
    pub fn project(&self) -> &Path {
        &self.project
    }

    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
            || self.focus.is_focused(window)
            || self
                .quick_add
                .as_ref()
                .is_some_and(|add| add.input.focus_handle(cx).is_focused(window))
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus.focus(window, cx);
    }

    /// Read the tasks again. Tasks that weren't there before are opened,
    /// and `true` returned, so the workspace can point them out.
    pub fn reload(&mut self, cx: &mut Context<Self>) -> bool {
        let read = match self.store.list() {
            Ok(tasks) => {
                self.error = None;
                Some(tasks)
            }
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                None
            }
        };
        let arrived = self.board.refresh(read);
        cx.notify();
        arrived
    }

    fn toggle_expanded(&mut self, id: &str, cx: &mut Context<Self>) {
        self.board.toggle(id);
        if self.quick_add.as_ref().is_some_and(|add| add.id == id) {
            self.quick_add = None;
        }
        cx.notify();
    }

    /// Show what saving a change came to: the tasks read again, and the
    /// error if it failed.
    fn changed(&mut self, saved: anyhow::Result<()>, cx: &mut Context<Self>) {
        if let Err(error) = saved {
            self.error = Some(format!("{error:#}"));
        }
        self.reload(cx);
    }

    pub fn toggle_todo(&mut self, id: &str, ix: usize, cx: &mut Context<Self>) {
        let saved = tasks::toggle_todo(&self.store, id, ix);
        self.changed(saved, cx);
    }

    pub fn delete(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Err(error) = self.store.discard(id) {
            self.error = Some(format!("Couldn't move it to the Trash: {error:#}"));
        }
        self.reload(cx);
    }

    /// Ask for the form for a new task.
    pub fn new_task(&mut self, cx: &mut Context<Self>) {
        self.quick_add = None;
        cx.emit(TaskPlannerEvent::Edit(None));
    }

    fn edit(&mut self, id: &str, cx: &mut Context<Self>) {
        self.quick_add = None;
        cx.emit(TaskPlannerEvent::Edit(Some(id.to_string())));
    }

    /// Hand task `id`, as it is on disk now, to the agent.
    fn implement(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.store.load(id) {
            Ok(task) => cx.emit(TaskPlannerEvent::Implement(task)),
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                cx.notify();
            }
        }
    }

    /// Hand todo `todo` of task `id`, as it is on disk now, to the agent.
    fn delegate(&mut self, id: &str, todo: usize, cx: &mut Context<Self>) {
        match tasks::delegated(&self.store, id, todo) {
            Ok(Some(task)) => cx.emit(TaskPlannerEvent::Delegate { task, todo }),
            // Gone since it was shown.
            Ok(None) => {
                self.reload(cx);
            }
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                cx.notify();
            }
        }
    }

    /// Read the tasks again and open `id`, e.g. one just saved.
    pub fn reveal(&mut self, id: &str, cx: &mut Context<Self>) {
        self.reload(cx);
        self.board.expand(id);
        cx.notify();
    }

    fn start_quick_add(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Add a todo"));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |this, input, event, window, cx| match event {
                    InputEvent::PressEnter { .. } => {
                        let text = input.read(cx).value().trim().to_string();
                        let Some(id) = this.quick_add.as_ref().map(|add| add.id.clone()) else {
                            return;
                        };
                        if text.is_empty() {
                            this.quick_add = None;
                            this.focus.focus(window, cx);
                            cx.notify();
                            return;
                        }
                        let saved = tasks::change(&this.store, &id, |task| {
                            task.todos.push(Todo::new(&text))
                        });
                        this.changed(saved, cx);
                        input.update(cx, |input, cx| input.set_value(String::new(), window, cx));
                    }
                    InputEvent::Blur if input.read(cx).value().trim().is_empty() => {
                        this.quick_add = None;
                        cx.notify();
                    }
                    _ => {}
                },
            );
        input.update(cx, |input, cx| input.focus(window, cx));
        self.quick_add = Some(QuickAdd {
            id: id.to_string(),
            input,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// A link in a task, clicked.
    fn open_link(&mut self, url: &str, markdown: &str, cx: &mut Context<Self>) {
        match links::target(url, markdown) {
            Some(Target::Web(url)) => cx.open_url(&url),
            Some(Target::Code(link)) => cx.emit(TaskPlannerEvent::Open {
                path: self.project.join(&link.path),
                link,
            }),
            None => {}
        }
    }

    fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.escape(window, cx);
    }

    /// Esc closes the todo field, else leaves the panel.
    pub fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.quick_add.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        } else {
            cx.emit(TaskPlannerEvent::Dismissed);
        }
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn is_expanded(&self, id: &str) -> bool {
        self.board.is_expanded(id)
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn click_link(&mut self, url: &str, markdown: &str, cx: &mut Context<Self>) {
        self.open_link(url, markdown, cx);
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn edit_task(&mut self, id: &str, cx: &mut Context<Self>) {
        self.edit(id, cx);
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn delegate_todo(&mut self, id: &str, todo: usize, cx: &mut Context<Self>) {
        self.delegate(id, todo, cx);
    }

    #[cfg(any(test, feature = "testing"))]
    pub fn start_adding_todo(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.start_quick_add(id, window, cx);
    }
}
