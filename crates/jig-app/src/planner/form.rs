//! New Task and Edit Task: a sheet over the window, like New Jig, with the
//! title, the description and the todos. The workspace shows it and hears
//! when it's saved or cancelled.
//!
//! Typing `@` in the description or a todo lists the project's names and
//! files ([`super::symbols`]); picking one puts in a link to it,
//! ``[`name`](path#L12)``.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui_kit::component::input::{
    Enter, Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp, Textarea,
    TextareaState,
};
use gpui_kit::component::{ActiveTheme as _, Icon, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_ai::tasks::{self, Task, Todo};

use super::symbols::{self, ProjectIndex, Symbol};
use super::{CROSS, checkbox, field, small_button};
use crate::git_panel::PLUS;

/// How many names the `@` list shows.
const MENTION_ROWS: usize = 8;

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

/// A field that can link to code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    Description,
    NewTodo,
    /// A todo, by its key.
    Todo(usize),
}

/// A field's state: one line or several.
enum FieldState {
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
}

impl FieldState {
    fn text_and_cursor(&self, cx: &App) -> (String, usize) {
        match self {
            FieldState::Line(state) => {
                let state = state.read(cx);
                (state.value().to_string(), state.cursor())
            }
            FieldState::Area(state) => {
                let state = state.read(cx);
                (state.value().to_string(), state.cursor())
            }
        }
    }

    /// Put `text` in place of `range`, as one undo step, the cursor after.
    fn replace(
        &self,
        range: std::ops::Range<usize>,
        text: &str,
        window: &mut Window,
        cx: &mut App,
    ) {
        match self {
            FieldState::Line(state) => state.update(cx, |state, cx| {
                state.set_selected_range(range, cx);
                state.replace(text.to_string(), window, cx);
            }),
            FieldState::Area(state) => state.update(cx, |state, cx| {
                state.set_selected_range(range, cx);
                state.replace(text.to_string(), window, cx);
            }),
        }
    }

    fn focus(&self, window: &mut Window, cx: &mut App) {
        match self {
            FieldState::Line(state) => state.update(cx, |state, cx| state.focus(window, cx)),
            FieldState::Area(state) => state.update(cx, |state, cx| state.focus(window, cx)),
        }
    }
}

/// Something `@` can link to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Mentioned {
    Symbol(Symbol),
    /// A file, relative to the project.
    File(String),
}

impl Mentioned {
    /// The Markdown link to it.
    fn link(&self) -> String {
        match self {
            Mentioned::Symbol(symbol) => {
                format!("[`{}`]({}#L{})", symbol.name, symbol.path, symbol.line)
            }
            Mentioned::File(path) => {
                let name = path.rsplit('/').next().unwrap_or(path);
                format!("[`{name}`]({path})")
            }
        }
    }
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
                .spawn(async move { symbols::index(&root) })
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
        let Some((at, query)) = mention_at(&text, cursor) else {
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
        let Some(index) = &self.index else {
            return Vec::new();
        };
        if query.is_empty() {
            return index
                .symbols
                .iter()
                .filter(|symbol| Some(&symbol.path) == self.current.as_ref())
                .take(MENTION_ROWS)
                .map(|symbol| Mentioned::Symbol(symbol.clone()))
                .collect();
        }
        let lower = query.to_lowercase();
        let names = crate::fuzzy::match_paths(
            index.symbols.iter().map(|symbol| symbol.name.as_str()),
            query,
            MENTION_ROWS * 4,
        );
        let files =
            crate::fuzzy::match_paths(index.files.iter().map(String::as_str), query, MENTION_ROWS);
        let mut scored: Vec<(i32, Mentioned)> = names
            .into_iter()
            .map(|found| {
                let symbol = &index.symbols[found.index];
                let name = symbol.name.to_lowercase();
                // A name that is, or starts with, what's typed beats one
                // that merely contains its letters.
                let bonus = if name == lower {
                    3000
                } else if name.starts_with(&lower) {
                    1500
                } else {
                    0
                };
                (found.score + bonus, Mentioned::Symbol(symbol.clone()))
            })
            .chain(files.into_iter().map(|found| {
                (
                    found.score,
                    Mentioned::File(index.files[found.index].clone()),
                )
            }))
            .collect();
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored
            .into_iter()
            .take(MENTION_ROWS)
            .map(|(_, item)| item)
            .collect()
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

    /// Esc closes the `@` list, else the form.
    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.mention.take().is_some() {
            cx.notify();
        } else {
            cx.emit(TaskFormEvent::Cancelled);
        }
    }

    #[cfg(test)]
    pub fn mention_items(&self) -> Option<Vec<String>> {
        self.mention
            .as_ref()
            .map(|mention| mention.items.iter().map(Mentioned::link).collect())
    }

    #[cfg(test)]
    pub fn index_ready(&self) -> bool {
        self.index.is_some()
    }

    #[cfg(test)]
    pub fn focus_description(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.description
            .update(cx, |input, cx| input.focus(window, cx));
    }

    #[cfg(test)]
    pub fn description(&self, cx: &App) -> String {
        self.description.read(cx).value().to_string()
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

/// The `@` and the name typed after it, when the cursor is right after
/// one: `See @par|` gives the `@`'s offset and `par`. An `@` inside a word,
/// like an email address's, isn't one.
fn mention_at(text: &str, cursor: usize) -> Option<(usize, &str)> {
    let before = text.get(..cursor)?;
    let at = before.rfind('@')?;
    let query = &before[at + 1..];
    let name = query
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '$' | ':'));
    let starts = before[..at]
        .chars()
        .next_back()
        .is_none_or(|c| c.is_whitespace() || matches!(c, '(' | '['));
    (name && starts).then_some((at, query))
}

impl TaskForm {
    /// The `@` list, under `field` when it's the one being typed in.
    fn render_mention(&self, field: Field, cx: &Context<Self>) -> Option<AnyElement> {
        let mention = self.mention.as_ref().filter(|m| m.field == field)?;
        let theme = cx.theme();
        let rows = mention.items.iter().enumerate().map(|(ix, item)| {
            let (kind, name, place) = match item {
                Mentioned::Symbol(symbol) => (
                    symbol.kind.clone(),
                    symbol.name.clone(),
                    format!("{}:{}", symbol.path, symbol.line),
                ),
                Mentioned::File(path) => (
                    "file".to_string(),
                    path.rsplit('/').next().unwrap_or(path).to_string(),
                    path.clone(),
                ),
            };
            h_flex()
                .id(("task-mention", ix))
                .h(px(26.))
                .px_2()
                .gap_2()
                .rounded(px(6.))
                .when(ix == mention.selected, |this| this.bg(theme.accent))
                .hover(|s| s.bg(theme.accent.opacity(0.6)))
                .child(
                    div()
                        .flex_none()
                        .w(px(44.))
                        .text_size(px(10.5))
                        .font_family(theme.mono_font_family.clone())
                        .text_color(theme.muted_foreground)
                        .child(kind),
                )
                .child(
                    div()
                        .flex_none()
                        .max_w(px(200.))
                        .truncate()
                        .text_size(px(12.5))
                        .font_family(theme.mono_font_family.clone())
                        .child(name),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(theme.muted_foreground)
                        .child(place),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.pick_mention(ix, window, cx);
                    }),
                )
        });
        let empty = mention.items.is_empty().then(|| {
            let text = if self.index.is_none() {
                "Looking through the project…"
            } else if mention.at + 1 == mention.end {
                "Type a name or a file."
            } else {
                "Nothing by that name."
            };
            div()
                .px_2()
                .py_1p5()
                .text_size(px(12.))
                .text_color(theme.muted_foreground)
                .child(text)
        });
        let list = jig_commands::surface::panel(cx)
            .w_full()
            .p_1()
            .flex()
            .flex_col()
            .children(rows)
            .children(empty)
            .child(div().px_2().pt_1().child(jig_commands::surface::hint(
                "↑↓ pick · ↩ links it · esc closes",
                cx,
            )));
        Some(
            div()
                .absolute()
                .top_full()
                .left_0()
                .right_0()
                .pt_1()
                .child(deferred(list).with_priority(1))
                .into_any_element(),
        )
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
        let todos = self.todos.iter().map(|todo| {
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
                    div()
                        .relative()
                        .flex_1()
                        .min_w_0()
                        .children(self.render_mention(Field::Todo(key), cx))
                        .child(field(
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
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .children(self.render_mention(Field::NewTodo, cx))
                    .child(field(
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
            .capture_action(cx.listener(Self::on_move_up))
            .capture_action(cx.listener(Self::on_move_down))
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_tab))
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
                    .relative()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(theme.foreground.opacity(0.12))
                    .bg(theme.foreground.opacity(0.03))
                    .text_size(px(12.5))
                    .child(
                        Textarea::new(&self.description)
                            .appearance(false)
                            .h(px(140.)),
                    )
                    .children(self.render_mention(Field::Description, cx)),
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

#[cfg(test)]
mod tests {
    use super::{Mentioned, Symbol, mention_at};

    #[test]
    fn finds_the_mention_before_the_cursor() {
        assert_eq!(mention_at("See @par", 8), Some((4, "par")));
        assert_eq!(mention_at("@", 1), Some((0, "")));
        assert_eq!(mention_at("(@Cache::get", 12), Some((1, "Cache::get")));
        assert_eq!(mention_at("See @par and", 12), None, "past a space");
        assert_eq!(mention_at("me@example.com", 14), None, "inside a word");
        assert_eq!(
            mention_at("See @par", 6),
            Some((4, "p")),
            "up to the cursor"
        );
    }

    #[test]
    fn links_name_their_place() {
        let symbol = Mentioned::Symbol(Symbol {
            name: "run".into(),
            kind: "fn".into(),
            path: "src/a.rs".into(),
            line: 3,
        });
        assert_eq!(symbol.link(), "[`run`](src/a.rs#L3)");
        assert_eq!(
            Mentioned::File("src/a.rs".into()).link(),
            "[`a.rs`](src/a.rs)"
        );
    }
}
