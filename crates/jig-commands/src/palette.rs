//! The floating command input that opens below the cursor.
//!
//! Typing filters the presets. Enter runs the highlighted row; once something
//! is typed, the last row always runs the text itself as a custom command.
//! Esc, or clicking away, dismisses it.

use std::rc::Rc;

use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::presets::{self, Invocation, Preset, Scope};

const MAX_ROWS: usize = 8;

pub enum PaletteEvent {
    Run(Invocation),
    Dismissed,
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Preset(usize),
    Custom(String),
}

pub struct CommandPalette {
    presets: Rc<Vec<Preset>>,
    input: Entity<InputState>,
    rows: Vec<Row>,
    selected: usize,
    has_selection: bool,
    _subscription: Subscription,
}

impl EventEmitter<PaletteEvent> for CommandPalette {}

impl Focusable for CommandPalette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl CommandPalette {
    /// `has_selection` decides whether a custom command rewrites the
    /// selection or inserts at the cursor.
    pub fn new(
        presets: Rc<Vec<Preset>>,
        has_selection: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = if has_selection {
            "Command for selection…"
        } else {
            "Command…"
        };
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => this.refilter(cx),
                    InputEvent::Blur => cx.emit(PaletteEvent::Dismissed),
                    _ => {}
                },
            );
        input.update(cx, |input, cx| input.focus(window, cx));
        let mut this = Self {
            presets,
            input,
            rows: Vec::new(),
            selected: 0,
            has_selection,
            _subscription: subscription,
        };
        this.refilter(cx);
        this
    }

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        self.rows = presets::filter(&self.presets, &query)
            .into_iter()
            .map(Row::Preset)
            .collect();
        if !query.trim().is_empty() {
            self.rows.push(Row::Custom(query.trim().to_string()));
        }
        self.selected = 0;
        cx.notify();
    }

    fn invocation(&self, row: &Row) -> Invocation {
        match row {
            Row::Preset(index) => Invocation::preset(&self.presets[*index]),
            Row::Custom(text) => Invocation::custom(text, self.has_selection),
        }
    }

    fn run(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get(index) {
            cx.emit(PaletteEvent::Run(self.invocation(row)));
        }
    }

    fn on_enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.run(self.selected, cx);
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        cx.emit(PaletteEvent::Dismissed);
    }

    fn on_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if !self.rows.is_empty() {
            self.selected = (self.selected + self.rows.len() - 1) % self.rows.len();
            cx.notify();
        }
    }

    fn on_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if !self.rows.is_empty() {
            self.selected = (self.selected + 1) % self.rows.len();
            cx.notify();
        }
    }

    fn render_row(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected = index == self.selected;
        let (label, scope): (SharedString, Option<Scope>) = match row {
            Row::Preset(i) => (
                self.presets[*i].name.clone().into(),
                Some(self.presets[*i].scope),
            ),
            Row::Custom(text) => (format!("Run “{text}”").into(), None),
        };
        h_flex()
            .id(("jig-command", index))
            .px_2()
            .py_1()
            .gap_2()
            .rounded_md()
            .justify_between()
            .text_sm()
            .text_color(theme.popover_foreground)
            .when(selected, |row| row.bg(theme.list_active))
            .when(!selected, |row| row.hover(|row| row.bg(theme.list_hover)))
            .child(div().truncate().child(label))
            .when_some(scope, |row, scope| {
                let scope = match scope {
                    Scope::Selection => "selection",
                    Scope::Cursor => "cursor",
                    Scope::File => "file",
                };
                row.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(scope),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.run(index, cx);
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
            .map(|(index, row)| self.render_row(index, row, cx).into_any_element())
            .collect();
        let theme = cx.theme();

        let palette = v_flex()
            .key_context("JigPalette")
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_up))
            .capture_action(cx.listener(Self::on_down))
            .w(px(380.))
            .p_1()
            .gap_0p5()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(px(10.))
            .shadow_lg()
            .child(Input::new(&self.input).appearance(false).cleanable(false))
            .when(!rows.is_empty(), |this| {
                this.child(div().h(px(1.)).mx_1().bg(theme.border))
                    .children(rows)
            });
        crate::motion::pop_in(palette, "jig-palette")
    }
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use gpui_kit::test::TestWindowExt;
    use gpui_kit::{
        AppContext, Bounds, Context, Entity, IntoElement, ParentElement, Point, Render, Styled,
        Subscription, TestAppContext, Window, WindowBounds, WindowOptions, div, px, size,
    };

    use super::{CommandPalette, PaletteEvent};
    use crate::presets::{self, Invocation, Scope};

    struct Host {
        palette: Entity<CommandPalette>,
        events: Vec<Option<Invocation>>,
        _subscription: Subscription,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.palette.clone())
        }
    }

    fn open(
        cx: &mut TestAppContext,
        has_selection: bool,
    ) -> (gpui_kit::AnyWindowHandle, Entity<Host>) {
        cx.update(gpui_kit::init);
        let handles = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(600.), px(400.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| {
                    let presets = Rc::new(presets::defaults());
                    let palette =
                        cx.new(|cx| CommandPalette::new(presets, has_selection, window, cx));
                    let subscription =
                        cx.subscribe(&palette, |host: &mut Host, _, event: &PaletteEvent, _| {
                            host.events.push(match event {
                                PaletteEvent::Run(invocation) => Some(invocation.clone()),
                                PaletteEvent::Dismissed => None,
                            })
                        });
                    Host {
                        palette,
                        events: Vec::new(),
                        _subscription: subscription,
                    }
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        handles
    }

    fn step(
        cx: &mut TestAppContext,
        window: gpui_kit::AnyWindowHandle,
        f: impl FnOnce(&mut Window, &mut gpui_kit::App),
    ) {
        cx.update_window(window, |_, window, cx| f(window, cx))
            .unwrap();
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn enter_runs_best_match(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("docs", cx);
        });
        step(cx, window, |window, cx| window.press("enter", cx));
        step(cx, window, |_, cx| {
            let events = &host.read(cx).events;
            assert_eq!(events.len(), 1);
            let invocation = events[0].clone().unwrap();
            assert_eq!(invocation.name.as_deref(), Some("Add docs"));
        });
    }

    #[gpui_kit::test]
    fn arrows_reach_custom_row(cx: &mut TestAppContext) {
        let (window, host) = open(cx, false);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("add", cx);
        });
        // "Add docs", "Add tests", then the custom row; up wraps to the last.
        step(cx, window, |window, cx| {
            window.press("up", cx);
            window.press("enter", cx);
        });
        step(cx, window, |_, cx| {
            let invocation = host.read(cx).events[0].clone().unwrap();
            assert_eq!(
                invocation,
                Invocation {
                    name: None,
                    instruction: "add".into(),
                    scope: Scope::Cursor
                }
            );
        });
    }

    #[gpui_kit::test]
    fn unmatched_text_runs_as_custom(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("make this async", cx);
        });
        step(cx, window, |window, cx| window.press("enter", cx));
        step(cx, window, |_, cx| {
            let invocation = host.read(cx).events[0].clone().unwrap();
            assert_eq!(invocation.name, None);
            assert_eq!(invocation.scope, Scope::Selection);
        });
    }

    #[gpui_kit::test]
    fn escape_dismisses(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("escape", cx);
        });
        step(cx, window, |_, cx| {
            assert_eq!(host.read(cx).events.first(), Some(&None))
        });
    }
}
