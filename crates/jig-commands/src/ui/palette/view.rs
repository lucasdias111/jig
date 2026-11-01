//! Drawing the input: the text with its completion, the rows, and the
//! model chip in the footer.

use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::{CONTEXT, CommandPalette, MAX_ROWS, Row, WIDTH};

impl CommandPalette {
    /// The chip at the footer's end: the agent and model in use, which ⌥M
    /// or a click changes for the window.
    fn render_chip(&self, accent: Hsla, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let menu = self.models();
        if menu.choices.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let label = menu
            .current()
            .map_or("Pick a model".to_string(), |choice| choice.label.clone());
        Some(
            h_flex()
                .id("jig-model-chip")
                .debug_selector(|| "palette-model-chip".into())
                .flex_none()
                .max_w(px(220.))
                .gap_1()
                .px_1p5()
                .rounded(px(5.))
                .text_size(px(11.))
                .text_color(theme.muted_foreground)
                .cursor_pointer()
                .hover(|this| this.bg(theme.foreground.opacity(0.06)).text_color(accent))
                .when(self.picking.is_some(), |this| {
                    this.bg(accent.opacity(0.14)).text_color(accent)
                })
                .child(div().truncate().child(label))
                .child(div().flex_none().child("⌥M"))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        cx.stop_propagation();
                        if this.picking.is_some() {
                            this.stop_picking(window, cx);
                        } else {
                            this.start_picking(window, cx);
                        }
                    }),
                ),
        )
    }

    /// Spotlight-sized input, marked with the agent's icon, with the rest
    /// of a command's name suggested after the typed text.
    fn render_input(&self, accent: Hsla, cx: &mut Context<Self>) -> impl IntoElement {
        let completion = self.completion(cx);
        let muted = cx.theme().muted_foreground;
        h_flex()
            .items_start()
            .gap_1()
            .px_1()
            .text_size(px(15.))
            .child(
                div().h(px(32.)).flex().items_center().child(
                    crate::ui::surface::agent_icon()
                        .size(px(15.))
                        .text_color(accent),
                ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .child(Textarea::new(&self.input).appearance(false))
                    .children(
                        completion
                            .as_ref()
                            .map(|rest| suggestion(self.input.clone(), rest.clone(), muted)),
                    ),
            )
            .when(completion.is_some(), |this| {
                this.child(
                    div()
                        .h(px(32.))
                        .flex()
                        .items_center()
                        .child(crate::ui::surface::hint("→ complete", cx)),
                )
            })
    }

    fn render_row(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = index == self.selected;
        let (label, detail): (SharedString, SharedString) = match row {
            Row::Message(text) => (format!("“{text}”").into(), "".into()),
            Row::Conversation(index) => {
                let conversation = &self.conversations[*index];
                (
                    conversation.title.clone().into(),
                    conversation.when.clone().into(),
                )
            }
            Row::Command(index) => {
                let command = &self.commands[*index];
                let kind = if command.skill { "skill" } else { "command" };
                (format!("/{}", command.name).into(), kind.into())
            }
            Row::Group(index) => {
                let menu = self.models();
                let group = menu.groups()[*index];
                let in_use = menu.current().is_some_and(|choice| choice.group == group);
                let count = menu
                    .choices
                    .iter()
                    .filter(|choice| choice.group == group)
                    .count();
                let detail = match (in_use, count) {
                    (true, _) => "in use".to_string(),
                    (false, 1) => "1 model".to_string(),
                    (false, count) => format!("{count} models"),
                };
                (group.to_string().into(), detail.into())
            }
            Row::Model(index) => {
                let menu = self.models();
                let choice = &menu.choices[*index];
                // Alone under its agent; in full among search results.
                let within = self
                    .picking
                    .as_ref()
                    .is_some_and(|picking| picking.group.is_some());
                let label = if within { &choice.name } else { &choice.label };
                (
                    label.clone().into(),
                    if menu.current == Some(*index) {
                        "in use".into()
                    } else {
                        "".into()
                    },
                )
            }
        };
        // What a command does, after its name.
        let description = match row {
            Row::Command(index) => Some(self.commands[*index].description.clone()),
            _ => None,
        }
        .filter(|description| !description.is_empty())
        .map(|description| {
            div()
                .min_w_0()
                .truncate()
                .text_size(px(12.))
                .opacity(0.7)
                .child(description)
        });
        let accent = crate::ui::surface::agent_accent(cx);
        let theme = cx.theme();
        // Selected like a macOS menu item: accent fill, white text.
        h_flex()
            .id(("jig-command", index))
            .h(px(28.))
            .px_2p5()
            .gap_2()
            .rounded(px(6.))
            .justify_between()
            .text_size(px(13.))
            .text_color(theme.popover_foreground)
            .when(selected, |row| {
                row.bg(accent).text_color(theme.primary_foreground)
            })
            .when(!selected, |row| {
                row.hover(|row| row.bg(theme.foreground.opacity(0.06)))
            })
            .child(
                h_flex()
                    .gap_1p5()
                    .min_w_0()
                    .child(
                        div()
                            .truncate()
                            .when(description.is_some(), |this| this.flex_none())
                            .child(label),
                    )
                    .children(description),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .when(selected, |this| {
                        this.text_color(theme.primary_foreground.opacity(0.8))
                    })
                    .when(!selected, |this| this.text_color(theme.muted_foreground))
                    .child(detail),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.choose(index, window, cx);
                }),
            )
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let first = self.selected.saturating_sub(MAX_ROWS - 1);
        let rows: Vec<_> = self
            .rows
            .clone()
            .iter()
            .enumerate()
            .skip(first)
            .take(MAX_ROWS)
            .flat_map(|(index, row)| {
                // Label each kind of row where it starts: the message, and
                // the commands or conversations below.
                let section = (index == first || self.rows[index - 1].section() != row.section())
                    .then(|| row.section());
                let header = section.map(|title| {
                    div()
                        .px_2p5()
                        .pt_1()
                        .pb_0p5()
                        .text_size(px(10.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(cx.theme().muted_foreground)
                        .child(title.to_uppercase())
                        .into_any_element()
                });
                header
                    .into_iter()
                    .chain([self.render_row(index, row, cx).into_any_element()])
            })
            .collect();
        let accent = crate::ui::surface::agent_accent(cx);
        let input = self.render_input(accent, cx).into_any_element();
        let chip = self
            .render_chip(accent, cx)
            .map(IntoElement::into_any_element);
        let theme = cx.theme();
        let footer = if let Some(picking) = &self.picking {
            match (&picking.group, self.rows.get(self.selected)) {
                (None, Some(Row::Group(_))) => "↩ its models · esc back".to_string(),
                (Some(_), _) if self.models().groups().len() > 1 => {
                    "↩ use it in this window · ⌫ back · esc done".to_string()
                }
                _ => "↩ use it in this window · esc back".to_string(),
            }
        } else {
            match self.rows.get(self.selected) {
                Some(Row::Conversation(_)) => "Picks up this conversation where it left off".into(),
                Some(Row::Command(_)) => {
                    "Runs in a new conversation · you review every edit".to_string()
                }
                _ => "Works across the project · you review every edit".to_string(),
            }
        };

        let palette = crate::ui::surface::panel(cx)
            .flex()
            .flex_col()
            .key_context(CONTEXT)
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_up))
            .capture_action(cx.listener(Self::on_down))
            .capture_action(cx.listener(Self::on_pick_model))
            .capture_action(cx.listener(Self::on_right))
            .capture_action(cx.listener(Self::on_end))
            .capture_action(cx.listener(Self::on_backspace))
            .w(px(WIDTH))
            .p_1p5()
            .gap_0p5()
            .border_color(accent.opacity(0.55))
            .child(input)
            .when(!rows.is_empty(), |this| {
                this.child(
                    div()
                        .h(px(1.))
                        .mx_neg_1p5()
                        .my_1()
                        .bg(theme.foreground.opacity(0.08)),
                )
                .children(rows)
            })
            .child(
                h_flex()
                    .px_2p5()
                    .pt_1()
                    .pb_0p5()
                    .gap_2()
                    .text_size(px(11.))
                    .text_color(accent)
                    .child(div().flex_1().min_w_0().truncate().child(footer))
                    .children(chip),
            );
        crate::ui::motion::pop_in(palette, "jig-palette")
    }
}

/// The rest of a command's name in grey after the typed text. Painted after the
/// input, so it goes where the input has just laid its text out, in the same
/// font at the size the input actually drew.
fn suggestion(input: Entity<TextareaState>, rest: String, color: Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |_, _, window, cx| {
            let state = input.read(cx);
            let typed = state.value();
            let (Some(drawn), Some(end)) = (
                state.range_to_bounds(&(0..typed.len())),
                state.range_to_bounds(&(typed.len()..typed.len())),
            ) else {
                return;
            };
            let style = window.text_style();
            let size = style.font_size.to_pixels(window.rem_size());
            let measured = window.text_system().shape_line(
                typed.clone(),
                size,
                &[style.to_run(typed.len())],
                None,
            );
            let size = if measured.width > px(0.) {
                size * (drawn.size.width / measured.width)
            } else {
                size
            };
            let run = TextRun {
                color,
                ..style.to_run(rest.len())
            };
            let line = window
                .text_system()
                .shape_line(rest.into(), size, &[run], None);
            let _ = line.paint(
                end.origin,
                end.size.height,
                TextAlign::Left,
                None,
                window,
                cx,
            );
        },
    )
    .absolute()
    .size_full()
}
