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
use crate::planner::links::{self, CodeLink};
use crate::planner::{TaskPlanner, TaskPlannerEvent};

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
            TaskPlannerEvent::Dismissed => this.focus_main(window, cx),
        });
        self.planner = PlannerState {
            view: Some(view),
            unseen: false,
            _events: Some(events),
            _watch: TaskWatch::start(root, window, cx),
        };
    }

    #[cfg(test)]
    pub(super) fn planner_view(&self) -> Option<&Entity<TaskPlanner>> {
        self.planner.view.as_ref()
    }

    pub(super) fn planner_shown(&self) -> bool {
        self.sidebar_shown() && self.sidebar_view == SidebarView::Tasks
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
        if !self.planner_shown() {
            self.toggle_sidebar_view(SidebarView::Tasks, window, cx);
        }
        if let Some(view) = self.planner.view.clone() {
            view.update(cx, |planner, cx| planner.new_task(window, cx));
        }
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
