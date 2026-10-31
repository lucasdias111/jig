//! Planner mode: the project's task planner, kept current by
//! watching `.jig/tasks/`, where agents add tasks too. A task arriving
//! while the view is hidden puts a dot on the title-bar Planner segment.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::*;
use jig_editor::EditorHandle as _;
use notify::Watcher as _;

use super::commands::Ask;
use super::mode::Mode;
use super::{NewTask, ToggleTaskPlanner, Workspace};
use jig_planner::{CodeLink, TaskForm, TaskFormEvent, first_code_link, locate_link};
use jig_planner::{TaskPlanner, TaskPlannerEvent};

/// An agent writes a task in one go, but saves come as several events.
const SETTLE: Duration = Duration::from_millis(100);

#[derive(Default)]
pub(super) struct PlannerState {
    view: Option<Entity<TaskPlanner>>,
    /// A task arrived while the view was hidden.
    pub(super) unseen: bool,
    _events: Option<Subscription>,
    _watch: Option<TaskWatch>,
}

/// New Task or Edit Task, as a sheet over the window, while it's open. Its
/// own entity, so the workspace only opens it, places it and shows the
/// task it saved.
#[derive(Default)]
pub(super) struct TaskFormSheet {
    open: Option<OpenTaskForm>,
}

struct OpenTaskForm {
    view: Entity<TaskForm>,
    _events: Subscription,
}

/// The form closed: it saved the task with this id, or was cancelled.
pub(super) enum TaskFormSheetEvent {
    Saved(String),
    Cancelled,
}

impl EventEmitter<TaskFormSheetEvent> for TaskFormSheet {}

impl TaskFormSheet {
    pub(super) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The form, while it's open.
    #[cfg(test)]
    pub(super) fn form(&self) -> Option<&Entity<TaskForm>> {
        self.open.as_ref().map(|open| &open.view)
    }

    /// Open the form on `project`, editing `task` or making a new one;
    /// `current` is the file on screen, whose names `@` lists first.
    fn open(
        &mut self,
        project: &Path,
        current: Option<String>,
        task: Option<jig_ai::tasks::Task>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.new(|cx| TaskForm::new(project, current, task, window, cx));
        let events = cx.subscribe(&view, |this, _, event, cx| {
            let event = match event {
                TaskFormEvent::Saved(id) => TaskFormSheetEvent::Saved(id.clone()),
                TaskFormEvent::Cancelled => TaskFormSheetEvent::Cancelled,
            };
            if this.close(cx) {
                cx.emit(event);
            }
        });
        self.open = Some(OpenTaskForm {
            view,
            _events: events,
        });
        cx.notify();
    }

    /// Close the form without saving. Returns whether it was open.
    pub(super) fn close(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.open.take().is_some();
        if open {
            cx.notify();
        }
        open
    }
}

impl Render for TaskFormSheet {
    /// The form, centred near the top like New Jig.
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let Some(open) = &self.open else {
            return div().into_any_element();
        };
        let width = px(jig_planner::FORM_WIDTH);
        let left = ((window.viewport_size().width - width) / 2.).max(px(8.));
        deferred(
            anchored()
                .position(point(left, px(super::TITLE_BAR_HEIGHT + 8.)))
                .snap_to_window_with_margin(px(8.))
                .child(open.view.clone()),
        )
        .into_any_element()
    }
}

/// Watches the project's folder, `.jig` and `.jig/tasks`, each on its own,
/// as they come to exist: cheap, unlike watching the whole project.
struct TaskWatch {
    watcher: notify::RecommendedWatcher,
    root: PathBuf,
    watching: Vec<PathBuf>,
    _task: Task<()>,
}

impl TaskWatch {
    /// Off in tests, whose own writes would race it.
    fn start(root: &Path, window: &mut Window, cx: &mut Context<Workspace>) -> Option<Self> {
        if cfg!(test) {
            return None;
        }
        let (sender, mut changes) = mpsc::unbounded::<()>();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            if event.is_ok_and(|event| !event.kind.is_access()) {
                let _ = sender.unbounded_send(());
            }
        })
        .ok()?;
        let task = cx.spawn_in(window, async move |this, cx| {
            while changes.next().await.is_some() {
                cx.background_executor().timer(SETTLE).await;
                while changes.try_recv().is_ok() {}
                if this
                    .update_in(cx, |this, window, cx| this.tasks_changed(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut watch = Self {
            watcher,
            root: root.to_path_buf(),
            watching: Vec::new(),
            _task: task,
        };
        watch.follow();
        Some(watch)
    }

    /// Watch whichever of the three folders exist and aren't watched yet.
    fn follow(&mut self) {
        let tasks = jig_ai::tasks::folder(&self.root);
        let folders = [
            self.root.clone(),
            tasks.parent().map(Path::to_path_buf).unwrap_or_default(),
            tasks,
        ];
        for folder in folders {
            if folder.is_dir()
                && !self.watching.contains(&folder)
                && self
                    .watcher
                    .watch(&folder, notify::RecursiveMode::NonRecursive)
                    .is_ok()
            {
                self.watching.push(folder);
            }
        }
    }
}

impl Workspace {
    /// The project changed: a planner for its tasks, watched.
    pub(super) fn start_planner(
        &mut self,
        root: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.new(|cx| TaskPlanner::new(root, cx));
        let events = cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
            TaskPlannerEvent::Open { path, link } => {
                let (path, link) = (path.clone(), link.clone());
                this.open_code_link(&path, &link, window, cx);
            }
            TaskPlannerEvent::Edit(id) => {
                let id = id.clone();
                this.open_task_form(id.as_deref(), window, cx);
            }
            TaskPlannerEvent::Delegate { task, todo } => {
                let (task, todo) = (task.clone(), *todo);
                this.delegate_todo(&task, todo, window, cx);
            }
            TaskPlannerEvent::Implement(task) => {
                let task = task.clone();
                this.implement_task(&task, window, cx);
            }
            TaskPlannerEvent::Dismissed => {
                this.set_mode(Mode::Code, window, cx);
                this.leave_sidebar(window, cx);
            }
        });
        self.planner = PlannerState {
            view: Some(view),
            unseen: false,
            _events: Some(events),
            _watch: TaskWatch::start(root, window, cx),
        };
        self.task_form.update(cx, |sheet, cx| sheet.close(cx));
    }

    /// The planner's task `id` shown, from a sketch's bar.
    pub(super) fn show_task(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !self.planner_shown() {
            self.set_mode(Mode::Planner, window, cx);
        }
        if let Some(view) = self.planner.view.clone() {
            view.update(cx, |planner, cx| planner.reveal(id, cx));
        }
    }

    #[cfg(test)]
    pub(super) fn planner_view(&self) -> Option<&Entity<TaskPlanner>> {
        self.planner.view.as_ref()
    }

    pub(super) fn planner_shown(&self) -> bool {
        self.mode == Mode::Planner
    }

    pub(super) fn planner_focused(&self, window: &Window, cx: &App) -> bool {
        self.planner_shown()
            && self
                .planner
                .view
                .as_ref()
                .is_some_and(|view| view.read(cx).is_focused(window, cx))
    }

    /// Something changed in the tasks folder, or near it.
    pub(super) fn tasks_changed(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(watch) = &mut self.planner._watch {
            watch.follow();
        }
        let Some(view) = self.planner.view.clone() else {
            return;
        };
        let arrived = view.update(cx, |planner, cx| planner.reload(cx));
        // A task made from a sketch links back to it.
        if let Some(map) = self.map.as_ref() {
            map.view.update(cx, |map, cx| {
                map.link_tasks();
                cx.notify();
            });
        }
        if arrived && !self.planner_shown() {
            self.planner.unseen = true;
            cx.notify();
        }
    }

    /// Planner has just shown.
    pub(super) fn show_planner_mode(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.planner.unseen = false;
        if let Some(view) = self.planner.view.clone() {
            view.update(cx, |planner, cx| {
                planner.reload(cx);
                planner.focus(window, cx);
            });
        }
    }

    pub(super) fn render_planner_view(&self) -> AnyElement {
        match &self.planner.view {
            Some(view) => view.clone().into_any_element(),
            None => div().into_any_element(),
        }
    }

    /// Restore Planner when returning from a linked file.
    pub(super) fn planner_at(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.is_some() {
            self.leave_tab(cx);
            self.switch_mode(Mode::Planner, cx);
            self.show_planner_mode(window, cx);
        }
    }

    pub(super) fn focus_planner_mode(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = &self.planner.view {
            view.update(cx, |planner, cx| planner.focus(window, cx));
        }
    }

    pub(super) fn render_planner_mode(&self, _: &Context<Self>) -> AnyElement {
        div()
            .id("planner-mode")
            .debug_selector(|| "planner-mode".into())
            .size_full()
            .child(self.render_planner_view())
            .into_any_element()
    }

    /// ⌃⇧T: toggle between Planner and Code.
    pub(super) fn toggle_task_planner(
        &mut self,
        _: &ToggleTaskPlanner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tree.is_none() {
            self.show_error("Open a project first, to plan its tasks.", window, cx);
            return;
        }
        let mode = if self.planner_shown() {
            Mode::Code
        } else {
            Mode::Planner
        };
        self.set_mode(mode, window, cx);
    }

    /// A new task's form over the window.
    pub(super) fn new_task(&mut self, _: &NewTask, window: &mut Window, cx: &mut Context<Self>) {
        if self.tree.is_none() {
            self.show_error("Open a project first, to plan its tasks.", window, cx);
            return;
        }
        self.open_task_form(None, window, cx);
    }

    /// New Task, or with `id`, Edit Task, as a sheet over the window.
    pub(super) fn open_task_form(
        &mut self,
        id: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = self
            .planner
            .view
            .as_ref()
            .map(|view| view.read(cx).project().to_path_buf())
        else {
            return;
        };
        let task = match id.map(|id| jig_ai::tasks::load(&project, id)).transpose() {
            Ok(task) => task,
            Err(error) => return self.show_error(&format!("{error:#}"), window, cx),
        };
        self.palette = None;
        // `@` with nothing typed lists the names in the file on screen.
        let current = (!self.home)
            .then(|| self.document().path.clone())
            .flatten()
            .and_then(|path| {
                path.strip_prefix(&project)
                    .ok()
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
            });
        self.task_form.update(cx, |sheet, cx| {
            sheet.open(&project, current, task, window, cx)
        });
        cx.notify();
    }

    /// The form closed: the keyboard goes back to the planner, or the main
    /// view, and a task it saved shows in the planner.
    pub(super) fn on_task_form_event(
        &mut self,
        _: &Entity<TaskFormSheet>,
        event: &TaskFormSheetEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match &self.planner.view {
            Some(view) if self.planner_shown() => {
                view.update(cx, |planner, cx| planner.focus(window, cx))
            }
            _ => self.focus_main(window, cx),
        }
        cx.notify();
        let TaskFormSheetEvent::Saved(id) = event else {
            return;
        };
        if !self.planner_shown() {
            self.set_mode(Mode::Planner, window, cx);
        }
        if let Some(view) = self.planner.view.clone() {
            view.update(cx, |planner, cx| {
                planner.reveal(id, cx);
                planner.focus(window, cx);
            });
        }
    }

    /// The form, while it's open.
    pub(super) fn render_task_form(&self, cx: &App) -> Option<AnyElement> {
        self.task_form
            .read(cx)
            .is_open()
            .then(|| self.task_form.clone().into_any_element())
    }

    #[cfg(test)]
    pub(super) fn task_form(&self, cx: &App) -> Option<Entity<TaskForm>> {
        self.task_form.read(cx).form().cloned()
    }

    /// Hand all of `task` to the agent: a conversation that works through
    /// its open todos in order, checking each off as it's done.
    pub(super) fn implement_task(
        &mut self,
        task: &jig_ai::tasks::Task,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.previewing() {
            return self.show_error(
                "Accept or reject the change under review first.",
                window,
                cx,
            );
        }
        self.start_conversation(implement_ask(task), window, cx);
    }

    /// Hand todo `todo` of `task` to the agent, the whole task as context:
    /// a conversation about the code the step links to, opened first, or
    /// else the selection.
    pub(super) fn delegate_todo(
        &mut self,
        task: &jig_ai::tasks::Task,
        todo: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(step) = task.todos.get(todo) else {
            return;
        };
        if self.previewing() {
            return self.show_error(
                "Accept or reject the change under review first.",
                window,
                cx,
            );
        }
        let project = self
            .planner
            .view
            .as_ref()
            .map(|view| view.read(cx).project().to_path_buf());
        if let Some((link, path)) = first_code_link(&step.text)
            .zip(project)
            .map(|(link, project)| {
                let path = project.join(&link.path);
                (link, path)
            })
            .filter(|(_, path)| path.is_file())
            && !self.showing(&path)
        {
            self.open_code_link(&path, &link, window, cx);
        }
        let mut name: String = step.text.chars().take(40).collect();
        if step.text.chars().count() > 40 {
            name.push('…');
        }
        let ask = Ask {
            name: Some(name),
            task: Some(task.step_context(todo)),
            ..Ask::new(step.text.clone())
        };
        self.start_conversation(ask, window, cx);
    }

    /// Open the code a task links to, remembering where we were for Go Back.
    pub(super) fn open_code_link(
        &mut self,
        path: &Path,
        link: &CodeLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A sketch on the Map, not its file.
        if let Some(sketch) = jig_map::sketch_of_link(&link.path) {
            return self.show_sketch(&sketch, window, cx);
        }
        if !path.is_file() {
            self.show_error(
                &format!("The task links to {}, which isn't there.", link.path),
                window,
                cx,
            );
            return;
        }
        self.remember_location(cx);
        self.open_file(path, window, cx);
        if !self.showing(path) {
            return;
        }
        let text = self.editor().text(cx);
        if let Some(range) = locate_link(&text, link) {
            self.editor().select(range, cx);
        }
        self.focus_editor(window, cx);
        cx.notify();
    }
}

/// What the agent is asked to implement all of `task`: the conversation is
/// named after it, and the whole task comes along as context.
pub(super) fn implement_ask(task: &jig_ai::tasks::Task) -> Ask {
    let instruction = if task.todos.is_empty() {
        format!("Implement the task “{}”.", task.title)
    } else {
        format!(
            "Implement the task “{}”: work through its open todos in order.",
            task.title
        )
    };
    Ask {
        name: Some(task.title.clone()),
        task: Some(task.context()),
        ..Ask::new(instruction)
    }
}
