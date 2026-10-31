//! New Task and Edit Task: a sheet over the window, like New Jig, with the
//! title, the description and the todos. The workspace shows it and hears
//! when it's saved or cancelled.
//!
//! Typing `@` in the description or a todo lists the project's names and
//! files ([`crate::domain::symbols`]); picking one puts in a link to it,
//! ``[`name`](path#L12)``.
//!
//! Drawing it is [`view`]'s.

mod fields;
#[cfg(any(test, feature = "testing"))]
mod testing;
mod view;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::input::{
    Enter, Escape, IndentInline, InputEvent, InputState, MoveDown, MoveUp, TextareaState,
};
use gpui_kit::*;
use jig_ai::tasks::{Task, Todo};

use crate::domain::mention::{self, Mentioned};
use crate::domain::symbols::{self, ProjectIndex};
use crate::domain::tasks;
use crate::store::persist::{ProjectFolder, TaskFolder};
use fields::{Field, FieldState};

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

/// The `@` list, while one is being typed.
struct Mention {
    field: Field,
    /// Where the `@` is, in bytes.
    at: usize,
    /// Where the typed name ends: the cursor.
    end: usize,
    items: Vec<Mentioned>,
    selected: usize,
}

pub struct TaskForm {
    project: PathBuf,
    /// The file open in the editor, relative to the project: `@` with
    /// nothing typed lists its names.
    current: Option<String>,
    /// The project's names and files, once read.
    index: Option<Arc<ProjectIndex>>,
    mention: Option<Mention>,
    _index_task: gpui_kit::Task<()>,
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
        current: Option<String>,
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
                .placeholder("Description, in Markdown. Type @ to link to code.")
                .default_value(task.description.clone())
        });
        let new_todo =
            cx.new(|cx| InputState::new(window, cx).placeholder("Add a todo; @ links to code"));
        let mut subscriptions = vec![
            cx.subscribe_in(&title, window, |this, _, event, window, cx| {
                if let InputEvent::PressEnter { .. } = event {
                    this.save(window, cx);
                }
            }),
            cx.subscribe_in(
                &description,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::PressEnter {
                        secondary: true, ..
                    } => this.save(window, cx),
                    _ => this.follow_mention(Field::Description, event, cx),
                },
            ),
            cx.subscribe_in(
                &new_todo,
                window,
                |this, _, event, window, cx| match event {
                    InputEvent::PressEnter { secondary, .. } => {
                        if *secondary {
                            this.save(window, cx);
                        } else {
                            this.add_todo(window, cx);
                        }
                    }
                    _ => this.follow_mention(Field::NewTodo, event, cx),
                },
            ),
        ];
        let mut todos = Vec::new();
        for (key, todo) in task.todos.iter().enumerate() {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(todo.text.clone()));
            subscriptions.push(Self::follow_todo(&input, key, window, cx));
            todos.push(FormTodo {
                key,
                input,
                done: todo.done,
            });
        }
        title.update(cx, |input, cx| input.focus(window, cx));
        let root = project.to_path_buf();
        let index_task = cx.spawn(async move |this, cx| {
            let index = cx
                .background_executor()
                .spawn(async move { symbols::index(&ProjectFolder::new(&root)) })
                .await;
            this.update(cx, |this, cx| {
                this.index = Some(Arc::new(index));
                this.refresh_mention(cx);
            })
            .ok();
        });
        Self {
            project: project.to_path_buf(),
            current,
            index: None,
            mention: None,
            _index_task: index_task,
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

    /// A todo's field: ⌘↩ saves, and `@` lists names.
    fn follow_todo(
        input: &Entity<InputState>,
        key: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Subscription {
        cx.subscribe_in(
            input,
            window,
            move |this, _, event, window, cx| match event {
                InputEvent::PressEnter {
                    secondary: true, ..
                } => this.save(window, cx),
                _ => this.follow_mention(Field::Todo(key), event, cx),
            },
        )
    }

    fn field_state(&self, field: Field) -> Option<FieldState> {
        match field {
            Field::Description => Some(FieldState::Area(self.description.clone())),
            Field::NewTodo => Some(FieldState::Line(self.new_todo.clone())),
            Field::Todo(key) => self
                .todos
                .iter()
                .find(|todo| todo.key == key)
                .map(|todo| FieldState::Line(todo.input.clone())),
        }
    }

    /// A field changed or took the keyboard: open, follow or close the
    /// `@` list.
    fn follow_mention(&mut self, field: Field, event: &InputEvent, cx: &mut Context<Self>) {
        match event {
            InputEvent::Change => self.mention_in(field, cx),
            // Another field: the list under the last one goes.
            InputEvent::Focus if self.mention.as_ref().is_some_and(|m| m.field != field) => {
                self.mention = None;
                cx.notify();
            }
            _ => {}
        }
    }

    /// Open or update the `@` list for what's typed before the cursor in
    /// `field`, or close it when that isn't a mention.
    fn mention_in(&mut self, field: Field, cx: &mut Context<Self>) {
        let Some(state) = self.field_state(field) else {
            return;
        };
        let (text, cursor) = state.text_and_cursor(cx);
        let Some((at, query)) = mention::mention_at(&text, cursor) else {
            if self.mention.take().is_some() {
                cx.notify();
            }
            return;
        };
        let items = self.matches(query);
        let selected = match &self.mention {
            Some(mention) if mention.field == field && mention.at == at => {
                mention.selected.min(items.len().saturating_sub(1))
            }
            _ => 0,
        };
        self.mention = Some(Mention {
            field,
            at,
            end: cursor,
            items,
            selected,
        });
        cx.notify();
    }

    /// The list again, e.g. once the project has been read.
    fn refresh_mention(&mut self, cx: &mut Context<Self>) {
        if let Some(field) = self.mention.as_ref().map(|mention| mention.field) {
            self.mention_in(field, cx);
        }
    }

    /// What `query` names: declarations and files, best first. With
    /// nothing typed, the names in the open file.
    fn matches(&self, query: &str) -> Vec<Mentioned> {
        match &self.index {
            Some(index) => mention::matches(index, self.current.as_deref(), query),
            None => Vec::new(),
        }
    }

    /// Put the link to item `ix` in place of the `@` and what was typed.
    fn pick_mention(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mention) = self.mention.take() else {
            return;
        };
        let Some(item) = mention.items.get(ix) else {
            return;
        };
        if let Some(state) = self.field_state(mention.field) {
            state.replace(mention.at..mention.end, &item.link(), window, cx);
            state.focus(window, cx);
        }
        // Putting it in reopened the list; it's done.
        self.mention = None;
        cx.notify();
    }

    /// While the `@` list is open, ↑↓ move in it, ↩ and Tab pick, and Esc
    /// closes it; otherwise the keys do what they always do.
    fn on_move_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(-1, cx);
    }

    fn on_move_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(1, cx);
    }

    fn move_selection(&mut self, by: isize, cx: &mut Context<Self>) {
        let Some(mention) = self.mention.as_mut().filter(|m| !m.items.is_empty()) else {
            cx.propagate();
            return;
        };
        let count = mention.items.len() as isize;
        mention.selected = (mention.selected as isize + by).rem_euclid(count) as usize;
        cx.stop_propagation();
        cx.notify();
    }

    fn on_enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_selected(window, cx);
    }

    fn on_tab(&mut self, _: &IndentInline, window: &mut Window, cx: &mut Context<Self>) {
        self.pick_selected(window, cx);
    }

    fn pick_selected(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(selected) = self
            .mention
            .as_ref()
            .filter(|m| !m.items.is_empty())
            .map(|m| m.selected)
        else {
            cx.propagate();
            return;
        };
        cx.stop_propagation();
        self.pick_mention(selected, window, cx);
    }

    /// The new todo's field, entered: it joins the list, and the field
    /// clears for the next.
    fn add_todo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.new_todo.read(cx).value().trim().to_string();
        if text.is_empty() {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).default_value(text));
        let key = self.next_key;
        self._subscriptions
            .push(Self::follow_todo(&input, key, window, cx));
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
        if self
            .mention
            .as_ref()
            .is_some_and(|m| m.field == Field::Todo(key))
        {
            self.mention = None;
        }
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
        let store = TaskFolder::new(&self.project);
        let saved = tasks::save_form(&store, self.id.clone(), title, description, todos);
        match saved {
            Ok(id) => cx.emit(TaskFormEvent::Saved(id)),
            Err(error) => {
                self.error = Some(format!("{error:#}"));
                cx.notify();
            }
        }
    }

    /// Esc closes the `@` list, else the form.
    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.mention.take().is_some() {
            cx.notify();
        } else {
            cx.emit(TaskFormEvent::Cancelled);
        }
    }
}
