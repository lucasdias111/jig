//! The sidebar's Tasks view: the project's task planner, kept current by
//! watching `.jig/tasks/`, where agents add tasks too. A task arriving
//! while the view is hidden puts a dot on its activity bar button.

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc;
use gpui_kit::*;
use jig_editor::EditorHandle as _;
use notify::Watcher as _;

use super::sidebar::SidebarView;
use super::{NewTask, ToggleTaskPlanner, Workspace};
use crate::planner::form::{TaskForm, TaskFormEvent};
use crate::planner::links::{self, CodeLink};
use crate::planner::{TaskPlanner, TaskPlannerEvent};
use jig_commands::presets::{Invocation, Scope};

/// An agent writes a task in one go, but saves come as several events.
const SETTLE: Duration = Duration::from_millis(100);

#[derive(Default)]
pub(super) struct PlannerState {
    view: Option<Entity<TaskPlanner>>,
    /// A task arrived while the view was hidden.
    pub(super) unseen: bool,
    _events: Option<Subscription>,
    _watch: Option<TaskWatch>,
    /// New Task or Edit Task, while it's open.
    pub(super) form: Option<OpenTaskForm>,
}

pub(super) struct OpenTaskForm {
    view: Entity<TaskForm>,
    _events: Subscription,
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
            TaskPlannerEvent::Delegate { task, todo, agent } => {
                let (task, todo, agent) = (task.clone(), *todo, *agent);
                this.delegate_todo(&task, todo, agent, window, cx);
            }
            TaskPlannerEvent::Implement(task) => {
                let task = task.clone();
                this.implement_task(&task, window, cx);
            }
            TaskPlannerEvent::Dismissed => this.leave_sidebar(window, cx),
        });
        self.planner = PlannerState {
            view: Some(view),
            unseen: false,
            _events: Some(events),
            _watch: TaskWatch::start(root, window, cx),
            form: None,
        };
    }

    #[cfg(test)]
    pub(super) fn planner_view(&self) -> Option<&Entity<TaskPlanner>> {
        self.planner.view.as_ref()
    }

    pub(super) fn planner_shown(&self) -> bool {
        self.sidebar_shown() && self.sidebar.view == SidebarView::Tasks
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
        if arrived && !self.planner_shown() {
            self.planner.unseen = true;
            cx.notify();
        }
    }

    /// The sidebar has just switched to Tasks.
    pub(super) fn show_planner_view(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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

    /// ⌃⇧T: the sidebar's Tasks view, or collapsing the sidebar.
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
        self.toggle_sidebar_view(SidebarView::Tasks, window, cx);
    }

    /// The Tasks view, with a new task's form open.
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
        self.new_command = None;
        // `@` with nothing typed lists the names in the file on screen.
        let current = (!self.home)
            .then(|| self.document().path.clone())
            .flatten()
            .and_then(|path| {
                path.strip_prefix(&project)
                    .ok()
                    .map(|path| path.to_string_lossy().replace('\\', "/"))
            });
        let view = cx.new(|cx| TaskForm::new(&project, current, task, window, cx));
        let events = cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
            TaskFormEvent::Saved(id) => {
                let id = id.clone();
                this.close_task_form(window, cx);
                if !this.planner_shown() {
                    this.toggle_sidebar_view(SidebarView::Tasks, window, cx);
                }
                if let Some(view) = this.planner.view.clone() {
                    view.update(cx, |planner, cx| {
                        planner.reveal(&id, cx);
                        planner.focus(window, cx);
                    });
                }
            }
            TaskFormEvent::Cancelled => this.close_task_form(window, cx),
        });
        self.planner.form = Some(OpenTaskForm {
            view,
            _events: events,
        });
        cx.notify();
    }

    pub(super) fn close_task_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.planner.form.take().is_some() {
            match &self.planner.view {
                Some(view) if self.planner_shown() => {
                    view.update(cx, |planner, cx| planner.focus(window, cx))
                }
                _ => self.focus_main(window, cx),
            }
            cx.notify();
        }
    }

    /// The task form, centred near the top like New Jig.
    pub(super) fn render_task_form(&self, window: &Window) -> Option<AnyElement> {
        let form = self.planner.form.as_ref()?;
        let width = px(crate::planner::form::WIDTH);
        let left = ((window.viewport_size().width - width) / 2.).max(px(8.));
        Some(
            deferred(
                anchored()
                    .position(point(left, px(super::TITLE_BAR_HEIGHT + 8.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(form.view.clone()),
            )
            .into_any_element(),
        )
    }

    #[cfg(test)]
    pub(super) fn task_form(&self) -> Option<Entity<TaskForm>> {
        self.planner.form.as_ref().map(|form| form.view.clone())
    }

    /// Hand todo `todo` of `task` to Jig mode or the agent, the whole task
    /// as context. Jig works on the code the step links to, in another
    /// file, else on the selection or the whole file; the agent starts a
    /// conversation about it.
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
        self.run_command(implement_invocation(task), window, cx);
    }

    pub(super) fn delegate_todo(
        &mut self,
        task: &jig_ai::tasks::Task,
        todo: usize,
        agent: bool,
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
        let mut whole_file = false;
        if let Some((link, path)) = links::first_code_link(&step.text)
            .zip(project)
            .map(|(link, project)| {
                let path = project.join(&link.path);
                (link, path)
            })
            .filter(|(_, path)| path.is_file())
            && !self.showing(&path)
        {
            self.open_code_link(&path, &link, window, cx);
            whole_file = true;
        }
        if self.home && !agent {
            return self.show_error(
                "Open the file this step is about first, for Jig to work in.",
                window,
                cx,
            );
        }
        let selected = !self.editor().selection(cx).is_empty();
        let scope = if agent || (selected && !whole_file) {
            Scope::Selection
        } else {
            Scope::File
        };
        let mut name: String = step.text.chars().take(40).collect();
        if step.text.chars().count() > 40 {
            name.push('…');
        }
        let invocation = Invocation {
            name: Some(name),
            instruction: step.text.clone(),
            scope,
            comment: None,
            agent,
            diagnostics: false,
            task: Some(task.step_context(todo)),
        };
        self.run_command(invocation, window, cx);
    }

    /// Open the code a task links to, remembering where we were for Go Back.
    pub(super) fn open_code_link(
        &mut self,
        path: &Path,
        link: &CodeLink,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        if let Some(range) = links::locate(&text, link) {
            self.editor().select(range, cx);
        }
        self.editor().focus(window, cx);
        cx.notify();
    }
}

/// What the agent is asked to implement all of `task`: the conversation is
/// named after it, and the whole task comes along as context.
pub(super) fn implement_invocation(task: &jig_ai::tasks::Task) -> Invocation {
    let instruction = if task.todos.is_empty() {
        format!("Implement the task “{}”.", task.title)
    } else {
        format!(
            "Implement the task “{}”: work through its open todos in order.",
            task.title
        )
    };
    Invocation {
        name: Some(task.title.clone()),
        instruction,
        scope: Scope::Selection,
        comment: None,
        agent: true,
        diagnostics: false,
        task: Some(task.context()),
    }
}
