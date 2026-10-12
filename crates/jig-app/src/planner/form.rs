//! New Task and Edit Task: a sheet over the window, like New Jig, with the
//! title, the description and the todos. The workspace shows it and hears
//! when it's saved or cancelled.

use std::path::{Path, PathBuf};

use gpui_kit::component::input::{Escape, Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Icon, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_ai::tasks::{self, Task, Todo};

use super::{CROSS, checkbox, field, small_button};
use crate::git_panel::PLUS;

/// The sheet's width, as New Jig's.
pub const WIDTH: f32 = 460.;

pub enum TaskFormEvent {
    /// Written to disk, as the task with this id.
    Saved(String),
    Cancelled,
}

struct FormTodo {
    /// Gives each todo's row an id of its own as rows come and go.
    key: usize,
    input: Entity<InputState>,
    done: bool,
}

pub struct TaskForm {
    project: PathBuf,
    /// `None` for a new task.
    id: Option<String>,
    title: Entity<InputState>,
    description: Entity<TextareaState>,
    todos: Vec<FormTodo>,
    /// The field for one more todo, at the end.
    new_todo: Entity<InputState>,
    next_key: usize,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TaskFormEvent> for TaskForm {}

impl Focusable for TaskForm {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.title.focus_handle(cx)
    }
}

impl TaskForm {
    /// The form for `task` in `project`, or for a new task.
    pub fn new(
        project: &Path,
        task: Option<Task>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
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
                    this.save(window, cx);
                }
            }),
            cx.subscribe_in(&description, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter {
                    secondary: true, ..
                } = event
                {
                    this.save(window, cx);
                }
            }),
            cx.subscribe_in(&new_todo, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { secondary, .. } = event {
                    if *secondary {
                        this.save(window, cx);
                    } else {
                        this.add_todo(window, cx);
                    }
                }
            }),
        ];
        let mut todos = Vec::new();
        for (key, todo) in task.todos.iter().enumerate() {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(todo.text.clone()));
            subscriptions.push(Self::save_on_secondary_enter(&input, window, cx));
            todos.push(FormTodo {
                key,
                input,
                done: todo.done,
            });
        }
        title.update(cx, |input, cx| input.focus(window, cx));
        Self {
            project: project.to_path_buf(),
            id: (!task.id.is_empty()).then_some(task.id),
            title,
            description,
            next_key: todos.len(),
            todos,
            new_todo,
            error: None,
            _subscriptions: subscriptions,
        }
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
                this.save(window, cx);
            }
        })
    }

    /// The new todo's field, entered: it joins the list, and the field
    /// clears for the next.
    fn add_todo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.new_todo.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
        self._subscriptions
            .push(Self::save_on_secondary_enter(&input, window, cx));
        self.todos.push(FormTodo {
            key: self.next_key,
            input,
            done: false,
        });
        self.next_key += 1;
        self.new_todo
            .update(cx, |input, cx| input.set_value(String::new(), window, cx));
        cx.notify();
    }

    fn remove_todo(&mut self, key: usize, cx: &mut Context<Self>) {
        self.todos.retain(|todo| todo.key != key);
        cx.notify();
    }

    fn toggle_todo(&mut self, key: usize, cx: &mut Context<Self>) {
        if let Some(todo) = self.todos.iter_mut().find(|todo| todo.key == key) {
            todo.done = !todo.done;
        }
        cx.notify();
    }

    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A todo typed but not entered still counts.
        self.add_todo(window, cx);
        let title = self.title.read(cx).value().trim().to_string();
        if title.is_empty() {
            self.error = Some("Give the task a title.".into());
            cx.notify();
            return;
        }
        let description = self.description.read(cx).value().trim().to_string();
        let todos: Vec<Todo> = self
            .todos
            .iter()
            .map(|todo| Todo {
                done: todo.done,
                ..Todo::new(&todo.input.read(cx).value())
            })
            .filter(|todo| !todo.text.is_empty())
            .collect();
        let saved = match self.id.clone() {
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
            Ok(id) => cx.emit(TaskFormEvent::Saved(id)),
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                cx.notify();
            }
        }
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        cx.emit(TaskFormEvent::Cancelled);
    }

    #[cfg(test)]
    pub fn fill(
        &mut self,
        title: &str,
        description: &str,
        todos: &[&str],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.title.update(cx, |input, cx| {
            input.set_value(title.to_string(), window, cx)
        });
        self.description.update(cx, |input, cx| {
            input.set_value(description.to_string(), window, cx)
        });
        for todo in todos {
            self.new_todo.update(cx, |input, cx| {
                input.set_value(todo.to_string(), window, cx)
            });
            self.add_todo(window, cx);
        }
    }
}

impl Render for TaskForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let label = |text: &'static str| {
            div()
                .pt_1()
                .text_xs()
                .font_weight(FontWeight::MEDIUM)
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let todos =
            self.todos.iter().map(|todo| {
                let key = todo.key;
                let group = SharedString::from(format!("form-todo-{key}"));
                h_flex()
                    .group(group.clone())
                    .gap_2()
                    .child(
                        checkbox(todo.done, cx)
                            .id(("form-todo-check", key))
                            .on_click(cx.listener(move |this, _, _, cx| this.toggle_todo(key, cx))),
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
                                            this.remove_todo(key, cx)
                                        })),
                                ),
                            cx,
                        )),
                    )
            });
        // In the checkboxes' column, so the field lines up with the todos.
        let new_todo = h_flex()
            .gap_2()
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
                    Input::new(&self.new_todo)
                        .appearance(false)
                        .cleanable(false),
                    cx,
                )),
            );
        let button = |id: &'static str, text: &'static str, primary: bool| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .px_3()
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
        let form = jig_commands::surface::panel(cx)
            .flex()
            .flex_col()
            .key_context("JigTaskForm")
            .capture_action(cx.listener(Self::on_escape))
            .w(px(WIDTH))
            .p_4()
            .gap_2()
            .text_size(px(13.))
            .child(
                div()
                    .text_size(px(15.))
                    .font_semibold()
                    .child(if self.id.is_some() {
                        "Edit Task"
                    } else {
                        "New Task"
                    }),
            )
            .child(label("Title"))
            .child(field(
                Input::new(&self.title).appearance(false).cleanable(false),
                cx,
            ))
            .child(label("Description"))
            .child(
                div()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(theme.foreground.opacity(0.12))
                    .bg(theme.foreground.opacity(0.03))
                    .text_size(px(12.5))
                    .child(
                        Textarea::new(&self.description)
                            .appearance(false)
                            .h(px(140.)),
                    ),
            )
            .child(label("Todo"))
            .child(
                // Many todos scroll rather than push the buttons off screen.
                v_flex()
                    .id("task-form-todos")
                    .max_h(px(220.))
                    .overflow_y_scroll()
                    .gap_1p5()
                    .children(todos)
                    .child(new_todo),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(div().text_xs().text_color(theme.danger).child(error))
            })
            .child(
                h_flex()
                    .pt_2()
                    .gap_2()
                    .child(jig_commands::surface::hint(
                        "↩ adds a todo · ⌘↩ saves · esc cancels",
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(
                        button("task-cancel", "Cancel", false)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(TaskFormEvent::Cancelled))),
                    )
                    .child(
                        button("task-save", "Save", true)
                            .on_click(cx.listener(|this, _, window, cx| this.save(window, cx))),
                    ),
            );
        jig_commands::motion::pop_in(form, "jig-task-form")
    }
}
