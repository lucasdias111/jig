//! Drawing the task list: the header, each task as a card, and an open
//! card's description and todos.

use gpui_kit::component::input::Input;
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, StyledExt as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_ai::tasks::Task;
use jig_commands::surface::PLUS_ICON as PLUS;

use super::TaskPlanner;
use crate::ui::widgets::{CHEVRON, CHEVRON_DOWN, PENCIL, TRASH, checkbox, field, small_button};

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
            .child(
                small_button("task-new", PLUS, "New Task", cx)
                    .on_click(cx.listener(|this, _, _, cx| this.new_task(cx))),
            )
            .into_any_element()
    }

    /// A task as a card: its title and progress, and when open, the
    /// description, the todos and its actions, each set apart.
    fn render_task(&self, ix: usize, task: &Task, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let open = self.board.is_expanded(&task.id);
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
        let group = SharedString::from(format!("task-{ix}"));
        // On hover, the actions take the progress pill's place. Shown and
        // hidden by visibility, not display: GPUI works the hover style out
        // again between prepaint and paint, and a child shown in between is
        // painted without having been prepainted, which panics.
        // A finished task has nothing left to implement.
        let implement = (!finished).then(|| {
            let id = id.clone();
            let accent = jig_commands::surface::agent_accent(cx);
            div()
                .id(("task-implement", ix))
                .flex_none()
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(5.))
                .text_color(accent)
                .hover(move |s| s.bg(accent.opacity(0.15)))
                .child(jig_commands::surface::agent_icon().size(px(12.)))
                .tooltip(|window, cx| {
                    Tooltip::new("Implement the task with the agent").build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.implement(&id, cx)
                }))
        });
        let actions = h_flex()
            .flex_none()
            .invisible()
            .group_hover(group.clone(), |s| s.visible())
            .children(implement)
            .child(
                small_button(("task-edit", ix), PENCIL, "Edit", cx).on_click({
                    let id = id.clone();
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.edit(&id, cx)
                    })
                }),
            )
            .child(
                small_button(("task-delete", ix), TRASH, "Move to Trash", cx).on_click({
                    let id = id.clone();
                    cx.listener(move |this, _, _, cx| {
                        cx.stop_propagation();
                        this.delete(&id, cx)
                    })
                }),
            );
        let progress = progress.map(|pill| {
            h_flex()
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .items_center()
                .group_hover(group.clone(), |s| s.invisible())
                .child(pill)
        });
        let slot = div()
            .relative()
            .flex_none()
            .child(actions)
            .children(progress);
        let header = h_flex()
            .id(("task", ix))
            .debug_selector(move || format!("task-{ix}"))
            .group(group)
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
            .child(slot)
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
            let group = SharedString::from(format!("todo-{ix}-{todo_ix}"));
            // Hand the step to the agent, the task with it.
            let delegate = {
                let id = id.clone();
                let accent = jig_commands::surface::agent_accent(cx);
                div()
                    .id(SharedString::from(format!("todo-agent-{ix}-{todo_ix}")))
                    .flex_none()
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .text_color(accent)
                    .hover(move |s| s.bg(accent.opacity(0.15)))
                    .child(jig_commands::surface::agent_icon().size(px(12.)))
                    .tooltip(|window, cx| Tooltip::new("Delegate to the agent").build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| this.delegate(&id, todo_ix, cx)))
            };
            let id = id.clone();
            h_flex()
                .group(group.clone())
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
                .when(!todo.done, |row| {
                    row.child(
                        h_flex()
                            .flex_none()
                            .invisible()
                            .group_hover(group, |s| s.visible())
                            .child(delegate),
                    )
                })
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
        let divider = || div().h(px(1.)).bg(theme.foreground.opacity(0.08));
        v_flex().child(divider()).child(
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
    }
}

impl Render for TaskPlanner {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let body = match () {
            () if self.board.tasks().is_empty() => div()
                .px_2()
                .py_3()
                .text_size(px(13.))
                .text_color(theme.muted_foreground)
                .child("No tasks yet. Make one with +, or ask the agent to plan something.")
                .into_any_element(),
            () => v_flex()
                .gap_2()
                .children(
                    self.board
                        .tasks()
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
