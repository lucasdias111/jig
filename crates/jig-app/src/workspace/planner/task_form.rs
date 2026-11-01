//! The New/Edit Task sheet over the window.

use super::*;

/// New Task or Edit Task, as a sheet over the window, while it's open. Its
/// own entity, so the workspace only opens it, places it and shows the
/// task it saved.
#[derive(Default)]
pub(in crate::workspace) struct TaskFormSheet {
    pub(super) open: Option<OpenTaskForm>,
}

pub(super) struct OpenTaskForm {
    pub(super) view: Entity<TaskForm>,
    pub(super) _events: Subscription,
}

/// The form closed: it saved the task with this id, or was cancelled.
pub(in crate::workspace) enum TaskFormSheetEvent {
    Saved(String),
    Cancelled,
}

impl EventEmitter<TaskFormSheetEvent> for TaskFormSheet {}

impl TaskFormSheet {
    pub(in crate::workspace) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The form, while it's open.
    #[cfg(test)]
    pub(in crate::workspace) fn form(&self) -> Option<&Entity<TaskForm>> {
        self.open.as_ref().map(|open| &open.view)
    }

    /// Open the form on `project`, editing `task` or making a new one;
    /// `current` is the file on screen, whose names `@` lists first.
    pub(super) fn open(
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
    pub(in crate::workspace) fn close(&mut self, cx: &mut Context<Self>) -> bool {
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
                .position(point(left, px(super::super::TITLE_BAR_HEIGHT + 8.)))
                .snap_to_window_with_margin(px(8.))
                .child(open.view.clone()),
        )
        .into_any_element()
    }
}

impl Workspace {
    /// A new task's form over the window.
    pub(in crate::workspace) fn new_task(
        &mut self,
        _: &NewTask,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tree.is_none() {
            self.show_error("Open a project first, to plan its tasks.", window, cx);
            return;
        }
        self.open_task_form(None, window, cx);
    }

    /// New Task, or with `id`, Edit Task, as a sheet over the window.
    pub(in crate::workspace) fn open_task_form(
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
    pub(in crate::workspace) fn on_task_form_event(
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
    pub(in crate::workspace) fn render_task_form(&self, cx: &App) -> Option<AnyElement> {
        self.task_form
            .read(cx)
            .is_open()
            .then(|| self.task_form.clone().into_any_element())
    }

    #[cfg(test)]
    pub(in crate::workspace) fn task_form(&self, cx: &App) -> Option<Entity<TaskForm>> {
        self.task_form.read(cx).form().cloned()
    }
}
