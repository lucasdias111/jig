//! The Tasks view of the sidebar (⌃⇧T): the project's tasks, each a
//! title, a description and a todo list, from `.jig/tasks/`. Click a task
//! to fold it open, check its todos off, add one, or edit and delete it.
//! Code in a task is linked; clicking a link opens the code, and Go Back
//! returns. Agents add tasks with their tool ([`jig_ai::tasks::mcp`]); the
//! workspace watches the folder, so theirs show up as they're made.

pub mod links;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_ai::tasks::{self, Task, Todo};

use crate::git_panel::PLUS;
use links::{CodeLink, Target};

/// Lucide "list-todo", for the activity bar.
pub const TASKS_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="5" width="6" height="6" rx="1"/><path d="m3 17 2 2 4-4"/><path d="M13 6h8"/><path d="M13 12h8"/><path d="M13 18h8"/></svg>"#;
/// Lucide "chevron-right".
const CHEVRON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>"#;
/// Lucide "chevron-down".
const CHEVRON_DOWN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;
/// Lucide "pencil".
const PENCIL: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21.174 6.812a1 1 0 0 0-3.986-3.987L3.842 16.174a2 2 0 0 0-.5.83l-1.321 4.352a.5.5 0 0 0 .623.622l4.353-1.32a2 2 0 0 0 .83-.497z"/><path d="m15 5 4 4"/></svg>"#;
/// Lucide "trash-2".
const TRASH: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6h18"/><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6"/><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2"/><line x1="10" x2="10" y1="11" y2="17"/><line x1="14" x2="14" y1="11" y2="17"/></svg>"#;
/// Lucide "x".
const CROSS: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"#;
const CHECK: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="3.5" stroke-linecap="round" stroke-linejoin="round"><path d="M20 6 9 17l-5-5"/></svg>"#;

pub enum TaskPlannerEvent {
    /// Open the code a link points at, in the file at this full path.
    Open {
        path: PathBuf,
        link: CodeLink,
    },
    Dismissed,
}

/// The task being made or edited.
struct TaskForm {
    /// `None` for a new task.
    id: Option<String>,
    title: Entity<InputState>,
    description: Entity<TextareaState>,
    todos: Vec<FormTodo>,
    /// The field for one more todo, at the end.
    new_todo: Entity<InputState>,
    /// Gives each todo's row an id of its own as rows come and go.
    next_key: usize,
    _subscriptions: Vec<Subscription>,
}

struct FormTodo {
    key: usize,
    input: Entity<InputState>,
    done: bool,
}

/// The field under an open task for adding a todo to it.
struct QuickAdd {
    id: String,
    input: Entity<InputState>,
    _subscription: Subscription,
}

pub struct TaskPlanner {
    project: PathBuf,
    tasks: Vec<Task>,
    /// Tasks folded open, by id.
    expanded: HashSet<String>,
    /// Every task seen, to open the ones that arrive (from an agent, say).
    known: Option<HashSet<String>>,
    form: Option<TaskForm>,
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
            tasks: Vec::new(),
            expanded: HashSet::new(),
            known: None,
            form: None,
            quick_add: None,
            error: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
        };
        this.reload(cx);
        this
    }

    #[cfg(test)]
    pub fn tasks(&self) -> &[Task] {
        &self.tasks
    }

    /// Whether the panel, or a field in it, has the keyboard.
    pub fn is_focused(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
            || self.focus.is_focused(window)
            || self.form.as_ref().is_some_and(|form| {
                form.title.focus_handle(cx).is_focused(window)
                    || form.description.focus_handle(cx).is_focused(window)
                    || form.new_todo.focus_handle(cx).is_focused(window)
                    || form
                        .todos
                        .iter()
                        .any(|todo| todo.input.focus_handle(cx).is_focused(window))
            })
            || self
                .quick_add
                .as_ref()
                .is_some_and(|add| add.input.focus_handle(cx).is_focused(window))
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.form {
            Some(form) => form.title.update(cx, |input, cx| input.focus(window, cx)),
            None => self.focus.focus(window, cx),
        }
    }

    /// Read the tasks again. Tasks that weren't there before are opened,
    /// and `true` returned, so the workspace can point them out.
    pub fn reload(&mut self, cx: &mut Context<Self>) -> bool {
        match tasks::list(&self.project) {
            Ok(tasks) => {
                self.tasks = tasks;
                self.error = None;
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        let present: HashSet<String> = self.tasks.iter().map(|task| task.id.clone()).collect();
        self.expanded.retain(|id| present.contains(id));
        let arrived = match &self.known {
            Some(known) => present.difference(known).cloned().collect(),
            None => Vec::new(),
        };
        self.expanded.extend(arrived.iter().cloned());
        self.known = Some(present);
        cx.notify();
        !arrived.is_empty()
    }

    fn toggle_expanded(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.expanded.remove(id) {
            self.expanded.insert(id.to_string());
        }
        if self.quick_add.as_ref().is_some_and(|add| add.id == id) {
            self.quick_add = None;
        }
        cx.notify();
    }

    /// Change the task `id` as it is on disk now, which an agent may have
    /// changed since it was shown, and save it.
    fn change(&mut self, id: &str, cx: &mut Context<Self>, change: impl FnOnce(&mut Task)) {
        let saved = tasks::load(&self.project, id).and_then(|mut task| {
            change(&mut task);
            tasks::save(&self.project, &task)
        });
        if let Err(error) = saved {
            self.error = Some(format!("{error:#}"));
        }
        self.reload(cx);
    }

    pub fn toggle_todo(&mut self, id: &str, ix: usize, cx: &mut Context<Self>) {
        self.change(id, cx, |task| {
            if let Some(todo) = task.todos.get_mut(ix) {
                todo.done = !todo.done;
            }
        });
    }

    pub fn delete(&mut self, id: &str, cx: &mut Context<Self>) {
        let deleted = tasks::path(&self.project, id)
            .and_then(|path| trash::delete(path).map_err(anyhow::Error::from));
        if let Err(error) = deleted {
            self.error = Some(format!("Couldn't move it to the Trash: {error:#}"));
        }
        self.reload(cx);
    }

    /// Open the form for a new task.
    pub fn new_task(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_form(None, window, cx);
    }

    fn edit(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        match tasks::load(&self.project, id) {
            Ok(task) => self.open_form(Some(task), window, cx),
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                cx.notify();
            }
        }
    }

    fn open_form(&mut self, task: Option<Task>, window: &mut Window, cx: &mut Context<Self>) {
        self.quick_add = None;
        let task = task.unwrap_or_default();
        let title = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Title")
                .default_value(task.title.clone())
        });
        let description = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder("Description, in Markdown. Link code as [`name`](src/file.rs#L12).")
                .default_value(task.description.clone())
        });
        let new_todo = cx.new(|cx| InputState::new(window, cx).placeholder("Add a todo"));
        let mut subscriptions = vec![
            cx.subscribe_in(&title, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.save_form(window, cx);
                }
            }),
            cx.subscribe_in(&description, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter {
                    secondary: true, ..
                } = event
                {
                    this.save_form(window, cx);
                }
            }),
            cx.subscribe_in(&new_todo, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { secondary, .. } = event {
                    if *secondary {
                        this.save_form(window, cx);
                    } else {
                        this.add_form_todo(window, cx);
                    }
                }
            }),
        ];
        let mut form_todos = Vec::new();
        for (key, todo) in task.todos.iter().enumerate() {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(todo.text.clone()));
            subscriptions.push(Self::save_on_secondary_enter(&input, window, cx));
            form_todos.push(FormTodo {
                key,
                input,
                done: todo.done,
            });
        }
        title.update(cx, |input, cx| input.focus(window, cx));
        self.form = Some(TaskForm {
            id: (!task.id.is_empty()).then_some(task.id),
            title,
            description,
            next_key: form_todos.len(),
            todos: form_todos,
            new_todo,
            _subscriptions: subscriptions,
        });
        self.error = None;
        cx.notify();
    }

    fn save_on_secondary_enter(
        input: &Entity<InputState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(input, window, |this, _, event, window, cx| {
            if let InputEvent::PressEnter {
                secondary: true, ..
            } = event
            {
                this.save_form(window, cx);
            }
        })
    }

    /// The new todo's field, entered: it joins the list, and the field
    /// clears for the next.
    fn add_form_todo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &mut self.form else {
            return;
        };
        let text = form.new_todo.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
        form._subscriptions
            .push(Self::save_on_secondary_enter(&input, window, cx));
        form.todos.push(FormTodo {
            key: form.next_key,
            input,
            done: false,
        });
        form.next_key += 1;
        form.new_todo
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        cx.notify();
    }

    fn remove_form_todo(&mut self, key: usize, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.form {
            form.todos.retain(|todo| todo.key != key);
        }
        cx.notify();
    }

    fn toggle_form_todo(&mut self, key: usize, cx: &mut Context<Self>) {
        if let Some(todo) = self
            .form
            .as_mut()
            .and_then(|form| form.todos.iter_mut().find(|todo| todo.key == key))
        {
            todo.done = !todo.done;
        }
        cx.notify();
    }

    pub fn save_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A todo typed but not entered still counts.
        self.add_form_todo(window, cx);
        let Some(form) = &self.form else {
            return;
        };
        let title = form.title.read(cx).value().trim().to_string();
        if title.is_empty() {
            self.error = Some("Give the task a title.".into());
            cx.notify();
            return;
        }
        let description = form.description.read(cx).value().trim().to_string();
        let todos: Vec<Todo> = form
            .todos
            .iter()
            .map(|todo| Todo {
                done: todo.done,
                ..Todo::new(&todo.input.read(cx).value())
            })
            .filter(|todo| !todo.text.is_empty())
            .collect();
        let saved = match form.id.clone() {
            Some(id) => tasks::save(
                &self.project,
                &Task {
                    id: id.clone(),
                    title,
                    description,
                    todos,
                },
            )
            .map(|()| id),
            None => tasks::create(&self.project, &title, &description, todos).map(|task| task.id),
        };
        match saved {
            Ok(id) => {
                self.form = None;
                self.reload(cx);
                self.expanded.insert(id);
                self.focus.focus(window, cx);
            }
            Err(error) => self.error = Some(format!("{error:#}")),
        }
        cx.notify();
    }

    fn cancel_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form = None;
        self.error = None;
        self.focus.focus(window, cx);
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
                        this.change(&id, cx, |task| task.todos.push(Todo::new(&text)));
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

    /// Esc closes the form or the todo field, else leaves the panel.
    pub fn escape(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.form.is_some() {
            self.cancel_form(window, cx);
        } else if self.quick_add.take().is_some() {
            self.focus.focus(window, cx);
            cx.notify();
        } else {
            cx.emit(TaskPlannerEvent::Dismissed);
        }
    }

    #[cfg(test)]
    pub fn form_open(&self) -> bool {
        self.form.is_some()
    }

    #[cfg(test)]
    pub fn is_expanded(&self, id: &str) -> bool {
        self.expanded.contains(id)
    }

    #[cfg(test)]
    pub fn fill_form(
        &mut self,
        title: &str,
        description: &str,
        todos: &[&str],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = &self.form else {
            return;
        };
        let (title_input, description_input, new_todo) = (
            form.title.clone(),
            form.description.clone(),
            form.new_todo.clone(),
        );
        title_input.update(cx, |input, cx| {
            input.set_value(title.to_string(), window, cx)
        });
        description_input.update(cx, |input, cx| {
            input.set_value(description.to_string(), window, cx)
        });
        for todo in todos {
            new_todo.update(cx, |input, cx| {
                input.set_value(todo.to_string(), window, cx)
            });
            self.add_form_todo(window, cx);
        }
    }

    #[cfg(test)]
    pub fn click_link(&mut self, url: &str, markdown: &str, cx: &mut Context<Self>) {
        self.open_link(url, markdown, cx);
    }

    #[cfg(test)]
    pub fn edit_task(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.edit(id, window, cx);
    }

    #[cfg(test)]
    pub fn start_adding_todo(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.start_quick_add(id, window, cx);
    }
}

/// Markdown that may hold links to code, rendered small for the sidebar.
fn markdown(
    id: impl Into<ElementId>,
    text: &str,
    planner: WeakEntity<TaskPlanner>,
    cx: &App,
) -> TextView {
    let theme = cx.theme();
    let code = StyleRefinement::default()
        .font_family(theme.mono_font_family.clone())
        .text_size(px(11.5));
    let source = text.to_string();
    // Headings in a task are labels, like Background or Risks.
    let inline_code = HighlightStyle {
        background_color: Some(theme.foreground.opacity(0.06)),
        ..Default::default()
    };
    TextView::markdown(id, text.to_string())
        .style(
            TextViewStyle::default()
                .paragraph_gap(rems(0.5))
                .heading_font_size(|_, _| px(12.))
                .inline_code(inline_code)
                .code_block(code),
        )
        .selectable(true)
        .on_link_click(move |url, _, _, cx| {
            planner
                .update(cx, |planner, cx| planner.open_link(url, &source, cx))
                .ok();
        })
}

impl TaskPlanner {
    fn render_header(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        h_flex()
            .h(px(26.))
            .pl_2()
            .pr_0p5()
            .gap_1()
            .child(
                div()
                    .flex_1()
                    .text_size(px(11.))
                    .font_semibold()
                    .text_color(theme.muted_foreground)
                    .child("Tasks"),
            )
            .when(self.form.is_none(), |this| {
                this.child(
                    small_button("task-new", PLUS, "New Task", cx)
                        .on_click(cx.listener(|this, _, window, cx| this.new_task(window, cx))),
                )
            })
            .into_any_element()
    }

    /// A task as a card: its title and progress, and when open, the
    /// description, the todos and its actions, each set apart.
    fn render_task(&self, ix: usize, task: &Task, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let open = self.expanded.contains(&task.id);
        let id = task.id.clone();
        let (done, total) = (task.done(), task.todos.len());
        let finished = total > 0 && done == total;
        let progress = (total > 0).then(|| {
            div()
                .flex_none()
                .px_1p5()
                .rounded_full()
                .text_size(px(11.))
                .map(|this| {
                    if finished {
                        this.bg(theme.primary.opacity(0.15))
                            .text_color(theme.primary)
                    } else {
                        this.bg(theme.foreground.opacity(0.07))
                            .text_color(theme.muted_foreground)
                    }
                })
                .child(format!("{done}/{total}"))
        });
        let header = h_flex()
            .id(("task", ix))
            .items_start()
            .px_2()
            .py(px(7.))
            .gap_1p5()
            .rounded(px(8.))
            .hover(|s| s.bg(theme.foreground.opacity(0.04)))
            .child(
                div().pt(px(4.)).child(
                    Icon::default()
                        .data(if open { CHEVRON_DOWN } else { CHEVRON })
                        .size(px(12.))
                        .flex_none()
                        .text_color(theme.muted_foreground),
                ),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .when(!open, |this| this.truncate())
                    .when(finished, |this| this.text_color(theme.muted_foreground))
                    .child(task.title.clone()),
            )
            .children(progress)
            .on_click({
                let id = id.clone();
                cx.listener(move |this, _, _, cx| this.toggle_expanded(&id, cx))
            });
        // Collapsed, a thin bar shows how far along it is.
        let bar = (!open && total > 0).then(|| {
            div()
                .mx_2()
                .mb(px(6.))
                .h(px(2.))
                .rounded_full()
                .bg(theme.foreground.opacity(0.08))
                .child(
                    div()
                        .h_full()
                        .rounded_full()
                        .w(relative(done as f32 / total as f32))
                        .bg(theme.primary),
                )
        });
        let card = v_flex()
            .rounded(px(9.))
            .border_1()
            .border_color(theme.foreground.opacity(if open { 0.14 } else { 0.09 }))
            .bg(theme.foreground.opacity(if open { 0.035 } else { 0.02 }))
            .child(header)
            .children(bar);
        if !open {
            return card.into_any_element();
        }
        card.child(self.render_task_body(ix, task, cx))
            .into_any_element()
    }

    fn render_task_body(&self, ix: usize, task: &Task, cx: &Context<Self>) -> Div {
        let theme = cx.theme();
        let id = task.id.clone();
        let planner = cx.entity().downgrade();
        let section = |text: String| {
            div()
                .text_size(px(10.5))
                .font_semibold()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let todos = task.todos.iter().enumerate().map(|(todo_ix, todo)| {
            let id = id.clone();
            h_flex()
                .items_start()
                .gap_2()
                .child(
                    div().pt(px(2.)).child(
                        checkbox(todo.done, cx)
                            .id(SharedString::from(format!("todo-{ix}-{todo_ix}")))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_todo(&id, todo_ix, cx)
                            })),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(12.5))
                        .when(todo.done, |this| {
                            this.text_color(theme.muted_foreground).line_through()
                        })
                        .child(markdown(
                            SharedString::from(format!("todo-text-{}-{todo_ix}", task.id)),
                            &todo.text,
                            planner.clone(),
                            cx,
                        )),
                )
        });
        let adding = self
            .quick_add
            .as_ref()
            .filter(|add| add.id == task.id)
            .map(|add| {
                field(
                    Input::new(&add.input).appearance(false).cleanable(false),
                    cx,
                )
            });
        let add_row = match adding {
            Some(field) => field.into_any_element(),
            None => h_flex()
                .id(("task-add-todo", ix))
                .gap_2()
                .text_size(px(12.))
                .text_color(theme.muted_foreground)
                .hover(|s| s.text_color(theme.foreground))
                .child(
                    div()
                        .w(px(14.))
                        .flex()
                        .justify_center()
                        .child(Icon::default().data(PLUS).size(px(12.))),
                )
                .child("Add a todo")
                .on_click({
                    let id = id.clone();
                    cx.listener(move |this, _, window, cx| this.start_quick_add(&id, window, cx))
                })
                .into_any_element(),
        };
        let action =
            |element_id: (&'static str, usize), icon: &'static [u8], label: &'static str| {
                h_flex()
                    .id(element_id)
                    .gap_1()
                    .px_1p5()
                    .py_0p5()
                    .rounded(px(5.))
                    .text_size(px(11.5))
                    .text_color(theme.muted_foreground)
                    .hover(|s| {
                        s.bg(theme.foreground.opacity(0.07))
                            .text_color(theme.foreground)
                    })
                    .child(Icon::default().data(icon).size(px(11.)))
                    .child(label)
            };
        let divider = || div().h(px(1.)).bg(theme.foreground.opacity(0.08));
        v_flex()
            .child(divider())
            .child(
                v_flex()
                    .px_3()
                    .py_2p5()
                    .gap_3()
                    .when(!task.description.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.foreground.opacity(0.8))
                                .child(markdown(
                                    SharedString::from(format!("task-description-{}", task.id)),
                                    &task.description,
                                    planner.clone(),
                                    cx,
                                )),
                        )
                    })
                    .child(
                        v_flex()
                            .gap_1p5()
                            .child(section(match task.todos.len() {
                                0 => "TODO".to_string(),
                                n => format!("TODO · {}/{n}", task.done()),
                            }))
                            .children(todos)
                            .child(add_row),
                    ),
            )
            .child(divider())
            .child(
                h_flex()
                    .px_2()
                    .py_1()
                    .gap_1()
                    .justify_end()
                    .child(action(("task-edit", ix), PENCIL, "Edit").on_click({
                        let id = id.clone();
                        cx.listener(move |this, _, window, cx| this.edit(&id, window, cx))
                    }))
                    .child(
                        action(("task-delete", ix), TRASH, "Move to Trash").on_click({
                            let id = id.clone();
                            cx.listener(move |this, _, _, cx| this.delete(&id, cx))
                        }),
                    ),
            )
    }

    fn render_form(&self, form: &TaskForm, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let label = |text: &'static str| {
            div()
                .pt_1()
                .text_size(px(11.))
                .font_semibold()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let todos = form.todos.iter().map(|todo| {
            let key = todo.key;
            let group = SharedString::from(format!("form-todo-{key}"));
            h_flex()
                .group(group.clone())
                .gap_1p5()
                .child(
                    checkbox(todo.done, cx)
                        .id(("form-todo-check", key))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.toggle_form_todo(key, cx)),
                        ),
                )
                .child(
                    div().flex_1().min_w_0().child(field(
                        Input::new(&todo.input)
                            .appearance(false)
                            .cleanable(false)
                            .suffix(
                                small_button(("form-todo-remove", key), CROSS, "Remove", cx)
                                    .invisible()
                                    .group_hover(group, |s| s.visible())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.remove_form_todo(key, cx)
                                    })),
                            ),
                        cx,
                    )),
                )
        });
        // In the checkboxes' column, so the field lines up with the todos.
        let new_todo = h_flex()
            .gap_1p5()
            .child(
                div()
                    .flex_none()
                    .size(px(14.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme.muted_foreground)
                    .child(Icon::default().data(PLUS).size(px(12.))),
            )
            .child(
                div().flex_1().min_w_0().child(field(
                    Input::new(&form.new_todo)
                        .appearance(false)
                        .cleanable(false),
                    cx,
                )),
            );
        let button = |id: &'static str, text: &'static str, primary: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .flex_1()
                .h(px(28.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(7.))
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .map(|this| {
                    if primary {
                        this.bg(theme.primary).text_color(theme.primary_foreground)
                    } else {
                        this.bg(theme.foreground.opacity(0.06))
                    }
                })
                .hover(|s| s.opacity(0.9))
                .child(text)
        };
        v_flex()
            .px_1()
            .gap_1p5()
            .child(label(if form.id.is_some() {
                "Edit Task"
            } else {
                "New Task"
            }))
            .child(field(
                Input::new(&form.title).appearance(false).cleanable(false),
                cx,
            ))
            .child(
                div()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(theme.foreground.opacity(0.12))
                    .bg(theme.foreground.opacity(0.03))
                    .text_size(px(12.5))
                    .child(
                        Textarea::new(&form.description)
                            .appearance(false)
                            .h(px(110.)),
                    ),
            )
            .child(label("Todo"))
            .children(todos)
            .child(new_todo)
            .child(
                h_flex()
                    .pt_1()
                    .gap_1p5()
                    .child(
                        button("task-cancel", "Cancel", false).on_click(
                            cx.listener(|this, _, window, cx| this.cancel_form(window, cx)),
                        ),
                    )
                    .child(
                        button("task-save", "Save", true).on_click(
                            cx.listener(|this, _, window, cx| this.save_form(window, cx)),
                        ),
                    ),
            )
            .child(jig_commands::surface::hint("⌘↩ saves · Esc cancels", cx))
            .into_any_element()
    }
}

impl Render for TaskPlanner {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let body = match &self.form {
            Some(form) => self.render_form(form, cx),
            None if self.tasks.is_empty() => div()
                .px_2()
                .py_3()
                .text_size(px(13.))
                .text_color(theme.muted_foreground)
                .child("No tasks yet. Make one with +, or ask the agent to plan something.")
                .into_any_element(),
            None => v_flex()
                .gap_2()
                .children(
                    self.tasks
                        .iter()
                        .enumerate()
                        .map(|(ix, task)| self.render_task(ix, task, cx)),
                )
                .into_any_element(),
        };
        v_flex()
            .id("task-planner")
            .key_context("JigTaskPlanner")
            .track_focus(&self.focus)
            .capture_action(cx.listener(Self::on_escape))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.escape(window, cx);
                }
            }))
            .size_full()
            .px_2()
            .gap_1()
            .child(self.render_header(cx))
            .child(
                div()
                    .id("task-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .pb_2()
                    .child(body),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .px_1()
                    .pb_2()
                    .text_size(px(11.5))
                    .text_color(theme.danger)
                    .child(error)
            }))
    }
}

/// An icon button the size of a row's.
fn small_button(
    id: impl Into<ElementId>,
    icon: &'static [u8],
    tooltip: &'static str,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    div()
        .id(id)
        .flex_none()
        .size(px(20.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .text_color(theme.muted_foreground)
        .hover(|s| {
            s.bg(theme.foreground.opacity(0.1))
                .text_color(theme.foreground)
        })
        .child(Icon::default().data(icon).size(px(12.)))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
}

/// A one-line field in the sidebar's style.
fn field(input: Input, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .w_full()
        .h(px(28.))
        .px_2()
        .flex()
        .items_center()
        .rounded(px(7.))
        .border_1()
        .border_color(theme.foreground.opacity(0.12))
        .bg(theme.foreground.opacity(0.03))
        .text_size(px(12.5))
        .child(input.w_full())
}

/// A small checkbox, like the Git view's.
fn checkbox(done: bool, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .flex_none()
        .size(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(if done {
            theme.primary
        } else {
            theme.foreground.opacity(0.25)
        })
        .map(|this| {
            if done {
                this.bg(theme.primary).child(
                    Icon::default()
                        .data(CHECK)
                        .size(px(10.))
                        .text_color(theme.primary_foreground),
                )
            } else {
                this.bg(theme.foreground.opacity(0.03))
            }
        })
}
