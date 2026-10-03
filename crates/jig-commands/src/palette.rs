//! The floating command input that opens below the cursor.
//!
//! Typing filters the presets. Enter runs the highlighted row; once something
//! is typed, the last row always runs the text itself as a custom command.
//! Commands set to take a note (and any command, with Tab) first switch the
//! input to a note step: Enter runs, Esc goes back to the list. Esc in the
//! list, or clicking away, dismisses it.

use std::rc::Rc;

use gpui_kit::component::input::{
    Backspace, Enter, Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveUp,
};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::presets::{self, CommentMode, Invocation, Preset};

const MAX_ROWS: usize = 8;

pub enum PaletteEvent {
    Run(Invocation),
    /// Cmd+Enter on typed text: turn it into a preset.
    SaveAsCommand(String),
    Dismissed,
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Preset(usize),
    Custom(String),
}

/// The note step for one preset.
struct NoteStep {
    preset: usize,
    /// The list query to restore when going back.
    query: String,
    /// Enter was pressed on an empty required note.
    missing: bool,
}

pub struct CommandPalette {
    presets: Rc<Vec<Preset>>,
    input: Entity<InputState>,
    rows: Vec<Row>,
    /// The query `rows` were built from.
    filtered_query: String,
    selected: usize,
    has_selection: bool,
    note: Option<NoteStep>,
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
        let input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(Self::list_placeholder(has_selection))
        });
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change if this.note.is_none() => this.refilter(cx),
                    InputEvent::Change => {
                        if let Some(note) = this.note.as_mut() {
                            note.missing = false;
                        }
                        cx.notify();
                    }
                    InputEvent::Blur => cx.emit(PaletteEvent::Dismissed),
                    _ => {}
                },
            );
        input.update(cx, |input, cx| input.focus(window, cx));
        let mut this = Self {
            presets,
            input,
            rows: Vec::new(),
            filtered_query: String::new(),
            selected: 0,
            has_selection,
            note: None,
            _subscription: subscription,
        };
        this.refilter(cx);
        this
    }

    fn list_placeholder(has_selection: bool) -> &'static str {
        if has_selection {
            "Command for selection…"
        } else {
            "Command…"
        }
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
        self.filtered_query = query;
        self.selected = 0;
        cx.notify();
    }

    /// Text typed in the same frame as Enter or Tab hasn't been filtered
    /// yet; do it now so the right row is chosen.
    fn ensure_filtered(&mut self, cx: &mut Context<Self>) {
        if self.query(cx) != self.filtered_query {
            self.refilter(cx);
        }
    }

    /// Run the row at `index`, or open its note step if the command asks
    /// for one.
    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.rows.get(index) {
            Some(Row::Preset(preset)) if self.presets[*preset].comment != CommentMode::None => {
                self.start_note(*preset, window, cx)
            }
            Some(Row::Preset(preset)) => cx.emit(PaletteEvent::Run(Invocation::preset(
                &self.presets[*preset],
            ))),
            Some(Row::Custom(text)) => cx.emit(PaletteEvent::Run(Invocation::custom(
                text,
                self.has_selection,
            ))),
            None => {}
        }
    }

    fn start_note(&mut self, preset: usize, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let command = &self.presets[preset];
        let placeholder = match (&command.comment_hint, command.comment) {
            (Some(hint), _) => format!("{hint}…"),
            (None, CommentMode::Required) => "Add a note…".to_string(),
            (None, _) => "Add a note (optional)…".to_string(),
        };
        self.note = Some(NoteStep {
            preset,
            query,
            missing: false,
        });
        self.input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.set_placeholder(placeholder, window, cx);
        });
        cx.notify();
    }

    fn back_to_list(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(note) = self.note.take() else { return };
        let placeholder = Self::list_placeholder(self.has_selection);
        self.input.update(cx, |input, cx| {
            input.set_value(note.query, window, cx);
            input.set_placeholder(placeholder, window, cx);
        });
        self.refilter(cx);
        if let Some(index) = self
            .rows
            .iter()
            .position(|row| *row == Row::Preset(note.preset))
        {
            self.selected = index;
        }
        cx.notify();
    }

    fn run_note(&mut self, cx: &mut Context<Self>) {
        let Some(note) = self.note.as_mut() else {
            return;
        };
        let preset = &self.presets[note.preset];
        let text = self.input.read(cx).value().to_string();
        if preset.comment == CommentMode::Required && text.trim().is_empty() {
            note.missing = true;
            cx.notify();
            return;
        }
        cx.emit(PaletteEvent::Run(
            Invocation::preset(preset).with_comment(&text),
        ));
    }

    fn on_enter(&mut self, action: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.note.is_some() {
            self.run_note(cx);
        } else if action.secondary {
            let query = self.query(cx);
            if !query.trim().is_empty() {
                cx.emit(PaletteEvent::SaveAsCommand(query.trim().to_string()));
            }
        } else {
            self.ensure_filtered(cx);
            self.choose(self.selected, window, cx);
        }
    }

    /// Tab adds a note to the highlighted command, whatever its setting.
    fn on_tab(&mut self, _: &IndentInline, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.note.is_none() {
            self.ensure_filtered(cx);
        }
        if self.note.is_none()
            && let Some(Row::Preset(preset)) = self.rows.get(self.selected)
        {
            self.start_note(*preset, window, cx);
        }
    }

    /// Backspace in an empty note goes back to the list.
    fn on_backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.note.is_some() && self.query(cx).is_empty() {
            cx.stop_propagation();
            self.back_to_list(window, cx);
        }
    }

    fn on_escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.note.is_some() {
            self.back_to_list(window, cx);
        } else {
            cx.emit(PaletteEvent::Dismissed);
        }
    }

    fn on_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.note.is_none() && !self.rows.is_empty() {
            self.selected = (self.selected + self.rows.len() - 1) % self.rows.len();
            cx.notify();
        }
    }

    fn on_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.note.is_none() && !self.rows.is_empty() {
            self.selected = (self.selected + 1) % self.rows.len();
            cx.notify();
        }
    }

    fn render_row(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let selected = index == self.selected;
        let (label, detail): (SharedString, SharedString) = match row {
            Row::Preset(i) => {
                let preset = &self.presets[*i];
                let detail = match preset.comment {
                    CommentMode::None if selected => format!("{} · ⇥ note", preset.scope.label()),
                    CommentMode::None => preset.scope.label().to_string(),
                    _ => format!("{} · note", preset.scope.label()),
                };
                (preset.name.clone().into(), detail.into())
            }
            Row::Custom(text) => (format!("Run “{text}”").into(), "⌘↩ save as command".into()),
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
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
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

    /// The command being annotated, shown above the note input.
    fn render_note_header(&self, note: &NoteStep, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let preset = &self.presets[note.preset];
        let requirement = match preset.comment {
            CommentMode::Required => "note required",
            _ => "note optional",
        };
        h_flex()
            .px_2()
            .pt_1()
            .gap_2()
            .text_xs()
            .child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded_md()
                    .bg(theme.primary.opacity(0.15))
                    .text_color(theme.popover_foreground)
                    .font_weight(FontWeight::MEDIUM)
                    .child(preset.name.clone()),
            )
            .child(
                div()
                    .text_color(if note.missing {
                        theme.danger
                    } else {
                        theme.muted_foreground
                    })
                    .child(format!("{} · {requirement}", preset.scope.label())),
            )
    }
}

impl Render for CommandPalette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let first = self.selected.saturating_sub(MAX_ROWS - 1);
        let rows: Vec<_> = if self.note.is_some() {
            Vec::new()
        } else {
            self.rows
                .clone()
                .iter()
                .enumerate()
                .skip(first)
                .take(MAX_ROWS)
                .map(|(index, row)| self.render_row(index, row, cx).into_any_element())
                .collect()
        };
        let header = self
            .note
            .as_ref()
            .map(|note| self.render_note_header(note, cx).into_any_element());
        let theme = cx.theme();

        let palette = v_flex()
            .key_context("JigPalette")
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_up))
            .capture_action(cx.listener(Self::on_down))
            .capture_action(cx.listener(Self::on_tab))
            .capture_action(cx.listener(Self::on_backspace))
            .w(px(380.))
            .p_1()
            .gap_0p5()
            .bg(theme.popover)
            .border_1()
            .border_color(theme.border)
            .rounded(px(10.))
            .shadow_lg()
            .children(header)
            .child(Input::new(&self.input).appearance(false).cleanable(false))
            .when(self.note.is_some(), |this| {
                this.child(
                    div()
                        .px_2()
                        .pb_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child("↩ run · esc back"),
                )
            })
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
        open_with(cx, has_selection, presets::defaults())
    }

    fn open_with(
        cx: &mut TestAppContext,
        has_selection: bool,
        presets: Vec<presets::Preset>,
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
                    let presets = Rc::new(presets);
                    let palette =
                        cx.new(|cx| CommandPalette::new(presets, has_selection, window, cx));
                    let subscription =
                        cx.subscribe(&palette, |host: &mut Host, _, event: &PaletteEvent, _| {
                            host.events.push(match event {
                                PaletteEvent::Run(invocation) => Some(invocation.clone()),
                                PaletteEvent::Dismissed | PaletteEvent::SaveAsCommand(_) => None,
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
                    comment: None,
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

    fn note_presets() -> Vec<presets::Preset> {
        presets::parse(
            r#"
            [[command]]
            name = "Create controller"
            scope = "cursor"
            prompt = "Insert a controller."
            comment = "required"
            comment_hint = "Entity name"

            [[command]]
            name = "Rename"
            prompt = "Rename this."
            comment = "optional"

            [[command]]
            name = "Simplify"
            prompt = "Simplify this."
            "#,
        )
        .unwrap()
    }

    fn events(cx: &mut TestAppContext, host: &Entity<Host>) -> Vec<Option<Invocation>> {
        cx.update(|cx| host.read(cx).events.clone())
    }

    fn in_note_step(cx: &mut TestAppContext, host: &Entity<Host>) -> bool {
        cx.update(|cx| host.read(cx).palette.read(cx).note.is_some())
    }

    #[gpui_kit::test]
    fn required_note_must_be_given(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, false, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("controller", cx);
            window.press("enter", cx);
        });
        assert!(in_note_step(cx, &host), "Enter asks for the note first");
        assert!(events(cx, &host).is_empty());

        step(cx, window, |window, cx| window.press("enter", cx));
        assert!(
            events(cx, &host).is_empty(),
            "an empty required note doesn't run"
        );

        step(cx, window, |window, cx| {
            window.input("User", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name.as_deref(), Some("Create controller"));
        assert_eq!(invocation.comment.as_deref(), Some("User"));
        assert_eq!(
            invocation.instruction, "Insert a controller.",
            "the base prompt is unchanged"
        );
    }

    #[gpui_kit::test]
    fn optional_note_can_be_skipped(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, true, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("rename", cx);
            window.press("enter", cx);
        });
        assert!(in_note_step(cx, &host));
        step(cx, window, |window, cx| window.press("enter", cx));
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name.as_deref(), Some("Rename"));
        assert_eq!(invocation.comment, None);
    }

    #[gpui_kit::test]
    fn tab_adds_a_note_to_any_command(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, true, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("simplify", cx);
            window.press("tab", cx);
        });
        assert!(in_note_step(cx, &host));
        step(cx, window, |window, cx| {
            window.input("keep the early return", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name.as_deref(), Some("Simplify"));
        assert_eq!(invocation.comment.as_deref(), Some("keep the early return"));
    }

    #[gpui_kit::test]
    fn escape_and_backspace_return_to_the_list(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, false, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("controller", cx);
            window.press("enter", cx);
        });
        step(cx, window, |window, cx| window.press("escape", cx));
        assert!(!in_note_step(cx, &host));
        assert!(
            events(cx, &host).is_empty(),
            "Esc in the note step doesn't dismiss"
        );
        assert_eq!(
            cx.update(|cx| host.read(cx).palette.read(cx).query(cx)),
            "controller",
            "the query is restored"
        );

        step(cx, window, |window, cx| window.press("enter", cx));
        assert!(in_note_step(cx, &host));
        step(cx, window, |window, cx| window.press("backspace", cx));
        assert!(!in_note_step(cx, &host));

        step(cx, window, |window, cx| window.press("escape", cx));
        assert_eq!(events(cx, &host), vec![None], "Esc in the list dismisses");
    }
}
