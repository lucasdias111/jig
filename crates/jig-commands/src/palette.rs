//! The floating command input that opens below the cursor.
//!
//! Typing filters the presets. Enter runs the highlighted row; once something
//! is typed, the last row always runs the text itself as a custom command.
//! Commands set to take a note (and any command, with Tab) first switch the
//! input to a note step: Enter runs, Esc goes back to the list. Esc in the
//! list, or clicking away, dismisses it.

use std::rc::Rc;

use gpui_kit::component::input::{
    Backspace, Enter, Escape, IndentInline, Input, InputEvent, InputState, MoveDown, MoveEnd,
    MoveRight, MoveUp,
};
use gpui_kit::component::{ActiveTheme as _, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::presets::{self, CommentMode, Invocation, Preset};

const MAX_ROWS: usize = 8;
const CONTEXT: &str = "JigPalette";

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
    /// What a quick command sees besides the instruction, e.g. "AGENTS.md".
    context: Vec<String>,
    /// Tab: hand this run to the coding agent.
    agent: bool,
    /// The text after a colon that follows a command's name, e.g. "terse"
    /// in "docs: terse". `None` without a colon or a matching command.
    inline_note: Option<String>,
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
            InputState::new(window, cx).placeholder(Self::list_placeholder(has_selection, false))
        });
        let subscription = cx.subscribe_in(
            &input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::Change if this.note.is_none() => this.on_query_changed(window, cx),
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
            context: Vec::new(),
            agent: false,
            inline_note: None,
            _subscription: subscription,
        };
        this.refilter(cx);
        this
    }

    fn list_placeholder(has_selection: bool, agent: bool) -> &'static str {
        match (agent, has_selection) {
            (false, true) => "Jig or prompt for the selection…",
            (false, false) => "Jig or prompt…",
            (true, true) => "Ask the agent about the selection…",
            (true, false) => "Ask the agent…",
        }
    }

    /// The highlighted command is set to run on the agent, whatever the
    /// switch says.
    fn agent_by_command(&self) -> bool {
        self.note
            .as_ref()
            .map(|note| note.preset)
            .or_else(|| match self.rows.get(self.selected) {
                Some(Row::Preset(preset)) => Some(*preset),
                _ => None,
            })
            .is_some_and(|preset| self.presets[preset].agent)
    }

    /// Whether Enter would hand the run to the agent.
    fn agent_lane(&self) -> bool {
        self.agent || self.agent_by_command()
    }

    pub fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    /// The project files a quick command is sent along with the file, named
    /// in the footer.
    pub fn with_context(mut self, context: Vec<String>) -> Self {
        self.context = context;
        self
    }

    fn on_query_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query(cx);
        // ":" right after choosing a command with the arrows completes its
        // name, so the note that follows goes to that command.
        if let Some(before) = query.strip_suffix(':')
            && before == self.filtered_query
            && !before.contains(':')
            && let Some(Row::Preset(preset)) = self.rows.get(self.selected)
        {
            let completed = format!("{}: ", self.presets[*preset].name);
            self.input
                .update(cx, |input, cx| input.set_value(completed, window, cx));
        }
        self.refilter(cx);
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        // "docs: terse" is the Add docs command with the note "terse", as
        // long as something matches "docs".
        let inline = query.split_once(':').and_then(|(name, note)| {
            let matches = presets::filter(&self.presets, name);
            (!name.trim().is_empty() && !matches.is_empty())
                .then(|| (matches, note.trim().to_string()))
        });
        let (matches, named) = match inline {
            Some((matches, note)) => {
                self.inline_note = Some(note);
                (matches, true)
            }
            None => {
                self.inline_note = None;
                let matches = presets::filter(&self.presets, &query);
                let named = matches
                    .first()
                    .is_some_and(|&first| presets::names(&self.presets[first], &query));
                (matches, named)
            }
        };
        // The typed text as a prompt first, then the commands it matches.
        // Enter takes the command only when the text clearly names it.
        self.rows = Vec::new();
        if !query.trim().is_empty() {
            self.rows.push(Row::Custom(query.trim().to_string()));
        }
        let has_prompt = !self.rows.is_empty();
        self.rows.extend(matches.into_iter().map(Row::Preset));
        self.filtered_query = query;
        self.selected = if named && has_prompt { 1 } else { 0 };
        cx.notify();
    }

    /// The rest of the highlighted jig's name when the text typed so far is
    /// how it starts and the cursor is at its end: "ain" after "Expl".
    fn completion(&self, cx: &App) -> Option<String> {
        let Some(Row::Preset(preset)) = self.rows.get(self.selected) else {
            return None;
        };
        let input = self.input.read(cx);
        let query = input.value();
        let at_end = input.selected_range() == (query.len()..query.len());
        if self.note.is_some() || !at_end || query.contains(':') {
            return None;
        }
        presets::completion(&self.presets[*preset].name, &query).map(str::to_string)
    }

    /// → or End at the end of the text completes the highlighted jig's name;
    /// anywhere else they move the cursor as usual.
    fn complete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ensure_filtered(cx);
        let Some(rest) = self.completion(cx) else {
            return;
        };
        cx.stop_propagation();
        self.input
            .update(cx, |input, cx| input.insert(rest, window, cx));
        self.refilter(cx);
    }

    fn on_right(&mut self, _: &MoveRight, window: &mut Window, cx: &mut Context<Self>) {
        self.complete(window, cx);
    }

    fn on_end(&mut self, _: &MoveEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.complete(window, cx);
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
        let note = self.inline_note.clone().unwrap_or_default();
        match self.rows.get(index) {
            Some(Row::Preset(preset))
                if self.presets[*preset].comment == CommentMode::Required && note.is_empty()
                    || self.presets[*preset].comment == CommentMode::Optional
                        && self.inline_note.is_none() =>
            {
                self.start_note(*preset, window, cx)
            }
            Some(Row::Preset(preset)) => cx.emit(PaletteEvent::Run(
                Invocation::preset(&self.presets[*preset])
                    .with_comment(&note)
                    .on_agent(self.agent),
            )),
            Some(Row::Custom(text)) => cx.emit(PaletteEvent::Run(
                Invocation::custom(text, self.has_selection).on_agent(self.agent),
            )),
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
        let placeholder = Self::list_placeholder(self.has_selection, self.agent);
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
            Invocation::preset(preset)
                .with_comment(&text)
                .on_agent(self.agent),
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

    /// Tab switches between the quick and the agent lane.
    fn on_tab(&mut self, _: &IndentInline, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.set_agent(!self.agent, window, cx);
    }

    /// Backspace in an empty note goes back to the list.
    fn on_backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.note.is_some() && self.query(cx).is_empty() {
            cx.stop_propagation();
            self.back_to_list(window, cx);
        }
    }

    fn set_agent(&mut self, agent: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.agent = agent;
        if self.note.is_none() {
            let placeholder = Self::list_placeholder(self.has_selection, agent);
            self.input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        cx.notify();
    }

    /// Quick | Agent, as a segmented control.
    fn render_lane_switch(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let agent = self.agent_lane();
        let segment = |lane: bool, cx: &mut Context<Self>| {
            let theme = cx.theme();
            let accent = crate::surface::lane_accent(lane, cx);
            let active = lane == agent;
            h_flex()
                .id(if lane {
                    "jig-lane-agent"
                } else {
                    "jig-lane-quick"
                })
                .gap_1()
                .px_2()
                .h(px(22.))
                .rounded(px(5.))
                .text_size(px(12.))
                .font_weight(FontWeight::MEDIUM)
                .child(
                    crate::surface::lane_icon(lane)
                        .size(px(12.))
                        .text_color(if active {
                            accent
                        } else {
                            theme.muted_foreground
                        }),
                )
                .child(crate::surface::lane_name(lane))
                .when(active, |this| {
                    this.bg(accent.opacity(0.16)).text_color(accent)
                })
                .when(!active, |this| {
                    this.text_color(theme.muted_foreground)
                        .hover(|this| this.bg(theme.foreground.opacity(0.06)))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.set_agent(lane, window, cx);
                    }),
                )
        };
        let right = if self.agent_by_command() && !self.agent {
            crate::surface::hint("set by this jig", cx)
        } else {
            crate::surface::hint("⇥ switch", cx)
        };
        h_flex()
            .px_1()
            .pb_0p5()
            .justify_between()
            .child(
                h_flex()
                    .p(px(2.))
                    .gap(px(2.))
                    .rounded(px(7.))
                    .bg(theme.foreground.opacity(0.05))
                    .child(segment(false, cx))
                    .child(segment(true, cx)),
            )
            .child(right)
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

    /// Spotlight-sized input, marked with the lane it runs on, with the rest
    /// of a jig's name suggested after the typed text.
    fn render_input(&self, agent: bool, accent: Hsla, cx: &mut Context<Self>) -> impl IntoElement {
        let completion = self.completion(cx);
        let muted = cx.theme().muted_foreground;
        div()
            .relative()
            .px_1()
            .text_size(px(15.))
            .child(
                Input::new(&self.input)
                    .appearance(false)
                    .cleanable(false)
                    .prefix(
                        crate::surface::lane_icon(agent)
                            .size(px(15.))
                            .text_color(accent),
                    )
                    .when(completion.is_some(), |this| {
                        this.suffix(crate::surface::hint("→ complete", cx))
                    }),
            )
            .children(completion.map(|rest| suggestion(self.input.clone(), rest, muted)))
    }

    fn render_row(&self, index: usize, row: &Row, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = index == self.selected;
        let (label, detail): (SharedString, SharedString) = match row {
            Row::Preset(i) => {
                let preset = &self.presets[*i];
                let detail = match preset.comment {
                    _ if self.inline_note.is_some() => preset.scope.label().to_string(),
                    CommentMode::None if selected => {
                        format!("{} · “:” adds a note", preset.scope.label())
                    }
                    CommentMode::None => preset.scope.label().to_string(),
                    _ => format!("{} · note", preset.scope.label()),
                };
                (preset.name.clone().into(), detail.into())
            }
            Row::Custom(text) => (
                format!("“{text}”").into(),
                if selected {
                    "⌘↩ save as jig".into()
                } else {
                    "".into()
                },
            ),
        };
        let row_agent = self.agent || matches!(row, Row::Preset(i) if self.presets[*i].agent);
        let accent = crate::surface::lane_accent(row_agent, cx);
        let theme = cx.theme();
        // Commands that always use the agent say so wherever they're listed.
        let tag = matches!(row, Row::Preset(i) if self.presets[*i].agent)
            .then(|| crate::surface::lane_tag(true, cx));
        // The note typed after the colon, shown with the command it goes to.
        let note = self
            .inline_note
            .clone()
            .filter(|note| !note.is_empty() && matches!(row, Row::Preset(_)))
            .map(|note| {
                div()
                    .px_1p5()
                    .rounded(px(4.))
                    .truncate()
                    .text_size(px(12.))
                    .bg(theme.foreground.opacity(0.07))
                    .when(selected, |this| {
                        this.bg(theme.primary_foreground.opacity(0.2))
                    })
                    .child(note)
            });
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
                            .when(note.is_some(), |this| this.flex_none())
                            .child(label),
                    )
                    .children(note)
                    .children(tag.map(|tag| {
                        tag.when(selected, |tag| {
                            tag.bg(theme.primary_foreground.opacity(0.2))
                                .text_color(theme.primary_foreground)
                        })
                    })),
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

    /// The command being annotated, shown above the note input.
    fn render_note_header(&self, note: &NoteStep, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let preset = &self.presets[note.preset];
        let requirement = match preset.comment {
            CommentMode::Required => "note required",
            _ => "note optional",
        };
        h_flex()
            .px_2p5()
            .pt_1p5()
            .gap_2()
            .text_size(px(11.))
            .child(
                div()
                    .px_1p5()
                    .py_0p5()
                    .rounded(px(5.))
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
                .flat_map(|(index, row)| {
                    // Label the two kinds of row: the typed prompt, and the
                    // commands below it.
                    let section = match row {
                        Row::Custom(_) => Some("Prompt"),
                        Row::Preset(_)
                            if index == 0 || matches!(self.rows[index - 1], Row::Custom(_)) =>
                        {
                            Some("Jigs")
                        }
                        Row::Preset(_) => None,
                    };
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
                .collect()
        };
        let header = self
            .note
            .as_ref()
            .map(|note| self.render_note_header(note, cx).into_any_element());
        let switch = self.render_lane_switch(cx).into_any_element();
        let agent = self.agent_lane();
        let accent = crate::surface::lane_accent(agent, cx);
        let input = self.render_input(agent, accent, cx).into_any_element();
        let theme = cx.theme();
        let footer = if agent {
            "Works across the project · you review every edit".to_string()
        } else {
            let target = if self.has_selection {
                "the selection"
            } else {
                "the cursor"
            };
            let sees = std::iter::once("this file")
                .chain(self.context.iter().map(String::as_str))
                .collect::<Vec<_>>()
                .join(", ");
            format!("Quick edit of {target} · sees {sees}")
        };

        let palette = crate::surface::panel(cx)
            .flex()
            .flex_col()
            .key_context(CONTEXT)
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_up))
            .capture_action(cx.listener(Self::on_down))
            .capture_action(cx.listener(Self::on_tab))
            .capture_action(cx.listener(Self::on_right))
            .capture_action(cx.listener(Self::on_end))
            .capture_action(cx.listener(Self::on_backspace))
            .w(px(420.))
            .p_1p5()
            .gap_0p5()
            .when(agent, |this| this.border_color(accent.opacity(0.55)))
            .child(switch)
            .children(header)
            .child(input)
            .when(self.note.is_some(), |this| {
                this.child(
                    div()
                        .px_2p5()
                        .pb_1()
                        .child(crate::surface::hint("↩ run · esc back", cx)),
                )
            })
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
                div()
                    .px_2p5()
                    .pt_1()
                    .pb_0p5()
                    .text_size(px(11.))
                    .text_color(if agent {
                        accent
                    } else {
                        theme.muted_foreground
                    })
                    .child(footer),
            );
        crate::motion::pop_in(palette, "jig-palette")
    }
}

/// The rest of a jig's name in grey after the typed text. Painted after the
/// input, so it goes where the input has just laid its text out, in the same
/// font at the size the input actually drew.
fn suggestion(input: Entity<InputState>, rest: String, color: Hsla) -> impl IntoElement {
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
        cx.update(|cx| {
            gpui_kit::init(cx);
        });
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
                    agent: false,
                    diagnostics: false,
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
            [[jig]]
            name = "Create controller"
            scope = "cursor"
            prompt = "Insert a controller."
            comment = "required"
            comment_hint = "Entity name"

            [[jig]]
            name = "Rename"
            prompt = "Rename this."
            comment = "optional"

            [[jig]]
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
    fn a_loose_match_runs_the_prompt(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, true, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            // A subsequence of "Simplify", but not its name: a prompt.
            window.input("smp", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name, None);
        assert_eq!(invocation.instruction, "smp");
    }

    #[gpui_kit::test]
    fn the_command_is_one_arrow_below_the_prompt(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, true, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("smp", cx);
        });
        step(cx, window, |window, cx| {
            window.press("down", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name.as_deref(), Some("Simplify"));
    }

    #[gpui_kit::test]
    fn a_colon_completes_the_command_chosen_with_the_arrows(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, true, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("smp", cx);
        });
        // The prompt is highlighted; ↓ picks Simplify.
        step(cx, window, |window, cx| window.press("down", cx));
        step(cx, window, |window, cx| window.input(":", cx));
        cx.update(|cx| {
            assert_eq!(host.read(cx).palette.read(cx).query(cx), "Simplify: ");
        });
        step(cx, window, |window, cx| {
            window.input("keep the early return", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name.as_deref(), Some("Simplify"));
        assert_eq!(invocation.comment.as_deref(), Some("keep the early return"));
    }

    fn query(cx: &mut TestAppContext, host: &Entity<Host>) -> String {
        cx.update(|cx| host.read(cx).palette.read(cx).query(cx))
    }

    #[gpui_kit::test]
    fn right_completes_the_named_jig(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| window.input("expl", cx));
        step(cx, window, |window, cx| window.press("right", cx));
        assert_eq!(query(cx, &host), "explain");
        // Typing goes on after the completed name, and it still runs.
        step(cx, window, |window, cx| window.input(":", cx));
        assert_eq!(query(cx, &host), "Explain: ");
    }

    #[gpui_kit::test]
    fn end_completes_in_the_agent_lane_too(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| window.press("tab", cx));
        step(cx, window, |window, cx| window.input("Add t", cx));
        step(cx, window, |window, cx| window.press("end", cx));
        assert_eq!(query(cx, &host), "Add tests");
    }

    #[gpui_kit::test]
    fn right_completes_the_jig_chosen_with_the_arrows(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| window.input("add", cx));
        step(cx, window, |window, cx| window.press("down", cx));
        step(cx, window, |window, cx| window.press("right", cx));
        assert_eq!(query(cx, &host), "add tests");
    }

    #[gpui_kit::test]
    fn right_moves_the_cursor_when_there_is_nothing_to_complete(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        // "xpl" only loosely matches Explain, so it stays a prompt.
        step(cx, window, |window, cx| window.input("xpl", cx));
        step(cx, window, |window, cx| window.press("right", cx));
        assert_eq!(query(cx, &host), "xpl");
        // Away from the end, → just moves the cursor.
        step(cx, window, |window, cx| {
            window.press("backspace", cx);
            window.press("backspace", cx);
            window.press("backspace", cx);
        });
        step(cx, window, |window, cx| window.input("expl", cx));
        step(cx, window, |window, cx| {
            window.press("left", cx);
            window.press("right", cx);
        });
        assert_eq!(query(cx, &host), "expl");
    }

    #[gpui_kit::test]
    fn a_colon_adds_a_note_to_any_command(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, true, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("simplify: keep the early return", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name.as_deref(), Some("Simplify"));
        assert_eq!(invocation.comment.as_deref(), Some("keep the early return"));
    }

    #[gpui_kit::test]
    fn a_colon_note_answers_a_command_that_asks_for_one(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, false, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("controller: Invoice", cx);
            window.press("enter", cx);
        });
        assert!(!in_note_step(cx, &host), "the note is already there");
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.comment.as_deref(), Some("Invoice"));
    }

    #[gpui_kit::test]
    fn a_colon_without_a_matching_command_is_just_text(cx: &mut TestAppContext) {
        let (window, host) = open_with(cx, true, note_presets());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("zzz: qqq", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert_eq!(invocation.name, None);
        assert_eq!(invocation.instruction, "zzz: qqq");
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

    #[gpui_kit::test]
    fn tab_switches_lanes(cx: &mut TestAppContext) {
        let (window, host) = open(cx, true);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("tab", cx);
            window.press("tab", cx);
        });
        cx.update(|cx| assert!(!host.read(cx).palette.read(cx).agent_lane()));
        step(cx, window, |window, cx| window.press("tab", cx));
        cx.update(|cx| {
            assert!(host.read(cx).palette.read(cx).agent_lane());
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("rename things", cx);
            window.press("enter", cx);
        });
        let invocation = events(cx, &host)[0].clone().unwrap();
        assert!(invocation.agent);
        assert_eq!(invocation.instruction, "rename things");
    }

    #[gpui_kit::test]
    fn an_agent_command_shows_the_agent_lane(cx: &mut TestAppContext) {
        let presets =
            presets::parse("[[jig]]\nname = \"Refactor\"\nprompt = \"p\"\nagent = true\n").unwrap();
        let (window, host) = open_with(cx, false, presets);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("refactor", cx);
        });
        cx.update(|cx| {
            let palette = host.read(cx).palette.read(cx);
            assert!(!palette.agent, "the switch itself stays put");
            assert!(palette.agent_lane(), "but the command runs on the agent");
        });
        step(cx, window, |window, cx| window.press("enter", cx));
        assert!(events(cx, &host)[0].clone().unwrap().agent);
    }
}
