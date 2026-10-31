//! Drawing the form: its fields, the todos, and the `@` list under the
//! field being typed in.

use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::{ActiveTheme as _, Icon, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_commands::surface::PLUS_ICON as PLUS;

use super::fields::Field;
use super::{TaskForm, TaskFormEvent, WIDTH};
use crate::domain::mention::Mentioned;
use crate::ui::widgets::{CROSS, checkbox, field, small_button};

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
