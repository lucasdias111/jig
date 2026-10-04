//! Choose a run configuration (⌃⌥R): a Spotlight-style panel listing the
//! project's configurations, filtered as you type. Choosing one runs it and
//! makes it the one ⌃R runs.

use std::ops::Range;

use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::fuzzy;
use crate::run_configs::RunConfig;

pub const WIDTH: f32 = 560.;
const ROW_HEIGHT: f32 = 34.;
const MAX_VISIBLE_ROWS: usize = 10;

pub const PLAY: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="black" stroke="black" stroke-width="1.5" stroke-linejoin="round"><path d="M7 4.5v15l12.5-7.5z"/></svg>"#;
const GEAR: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z"/></svg>"#;

pub enum RunPickerEvent {
    /// Run this configuration, by its index in the list given.
    Run(usize),
    /// Open `.jig/run.toml`, creating it if need be.
    Edit,
    Dismissed,
    Blurred,
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Config {
        index: usize,
        /// Byte offsets of the name's characters the query matched.
        positions: Vec<usize>,
    },
    Edit,
}

pub struct RunPicker {
    input: Entity<InputState>,
    configs: Vec<RunConfig>,
    /// Still asking the project what it can run.
    loading: bool,
    /// The configuration ⌃R runs, by name.
    current: Option<String>,
    rows: Vec<Row>,
    filtered_query: String,
    selected: usize,
    scroll: UniformListScrollHandle,
    _subscription: Subscription,
}

impl EventEmitter<RunPickerEvent> for RunPicker {}

impl Focusable for RunPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl RunPicker {
    pub fn new(
        configs: Vec<RunConfig>,
        loading: bool,
        current: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Run configuration…"));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, _, cx| match event {
            InputEvent::Change => this.refilter(cx),
            InputEvent::Blur => cx.emit(RunPickerEvent::Blurred),
            _ => {}
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        let mut this = Self {
            input,
            configs,
            loading,
            current,
            rows: Vec::new(),
            filtered_query: String::new(),
            selected: 0,
            scroll: UniformListScrollHandle::default(),
            _subscription: subscription,
        };
        this.refilter(cx);
        this
    }

    /// The configurations as loaded; the list given before may have been an
    /// old one.
    pub fn set_configs(&mut self, configs: Vec<RunConfig>, cx: &mut Context<Self>) {
        self.configs = configs;
        self.loading = false;
        self.refilter(cx);
    }

    #[cfg(test)]
    pub fn row_names(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| match row {
                Row::Config { index, .. } => self.configs[*index].name.clone(),
                Row::Edit => "Edit run.toml…".into(),
            })
            .collect()
    }

    fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let mut rows: Vec<Row> = if query.trim().is_empty() {
            (0..self.configs.len())
                .map(|index| Row::Config {
                    index,
                    positions: Vec::new(),
                })
                .collect()
        } else {
            fuzzy::match_paths(
                self.configs.iter().map(|c| c.name.as_str()),
                &query,
                usize::MAX,
            )
            .into_iter()
            .map(|m| Row::Config {
                index: m.index,
                positions: m.positions,
            })
            .collect()
        };
        rows.push(Row::Edit);
        // Start on the current configuration, so ↩ reruns it.
        self.selected = if query.trim().is_empty() {
            rows.iter()
                .position(|row| {
                    matches!(row, Row::Config { index, .. }
                        if Some(&self.configs[*index].name) == self.current.as_ref())
                })
                .unwrap_or(0)
        } else {
            0
        };
        self.rows = rows;
        self.filtered_query = query;
        self.scroll
            .scroll_to_item(self.selected, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        match self.rows.get(index) {
            Some(Row::Config { index, .. }) => cx.emit(RunPickerEvent::Run(*index)),
            Some(Row::Edit) => cx.emit(RunPickerEvent::Edit),
            None => {}
        }
    }

    fn on_enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if self.query(cx) != self.filtered_query {
            self.refilter(cx);
        }
        self.choose(self.selected, cx);
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        cx.emit(RunPickerEvent::Dismissed);
    }

    fn on_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        let len = self.rows.len();
        self.select((self.selected + len - 1) % len, cx);
    }

    fn on_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.select((self.selected + 1) % self.rows.len(), cx);
    }

    fn select(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = index;
        self.scroll.scroll_to_item(index, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn render_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let selected = ix == self.selected;
        let (text, muted) = if selected {
            (
                theme.primary_foreground,
                theme.primary_foreground.opacity(0.75),
            )
        } else {
            (theme.popover_foreground, theme.muted_foreground)
        };
        let (icon, name, detail, tag) = match &self.rows[ix] {
            Row::Config { index, positions } => {
                let config = &self.configs[*index];
                let mark = HighlightStyle {
                    color: (!selected).then_some(theme.primary),
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                };
                let highlights: Vec<(Range<usize>, HighlightStyle)> = positions
                    .iter()
                    .map(|&i| {
                        let len = config.name[i..].chars().next().map_or(1, char::len_utf8);
                        (i..i + len, mark)
                    })
                    .collect();
                let current = Some(&config.name) == self.current.as_ref();
                (
                    PLAY,
                    StyledText::new(SharedString::from(config.name.clone()))
                        .with_highlights(highlights),
                    config.command.clone(),
                    if current {
                        "current".to_string()
                    } else {
                        config.source.label().to_string()
                    },
                )
            }
            Row::Edit => (
                GEAR,
                StyledText::new("Edit run.toml…"),
                crate::run_configs::FILE.to_string(),
                String::new(),
            ),
        };
        h_flex()
            .id(("run-picker-row", ix))
            .w_full()
            .overflow_hidden()
            .h(px(ROW_HEIGHT))
            .px_2p5()
            .gap_2p5()
            .rounded(px(7.))
            .text_color(text)
            .when(selected, |row| row.bg(theme.primary))
            .when(!selected, |row| {
                row.hover(|row| row.bg(theme.foreground.opacity(0.06)))
            })
            .child(
                Icon::default()
                    .data(icon)
                    .size(px(14.))
                    .flex_none()
                    .text_color(if selected {
                        text
                    } else {
                        theme.success.opacity(0.9)
                    }),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .items_baseline()
                    .child(
                        div()
                            .flex_none()
                            .max_w(relative(0.5))
                            .truncate()
                            .text_size(px(13.5))
                            .child(name),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(muted)
                            .font_family(theme.mono_font_family.clone())
                            .child(detail),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(muted)
                    .child(tag),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.choose(ix, cx);
                }),
            )
            .into_any_element()
    }
}

impl Render for RunPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.rows.len();
        let list = uniform_list(
            "run-picker-rows",
            rows,
            cx.processor(|this, range: Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .w_full()
        .h(px(rows.min(MAX_VISIBLE_ROWS) as f32 * ROW_HEIGHT));

        let empty = (self.configs.is_empty() || rows == 1).then(|| {
            let message = if self.loading {
                "Looking for what this project can run…".to_string()
            } else if self.configs.is_empty() {
                "Nothing found to run. Add a configuration to run.toml.".to_string()
            } else {
                format!("No configurations match “{}”", self.filtered_query.trim())
            };
            div()
                .px_2p5()
                .py_3()
                .text_size(px(13.))
                .text_color(theme.muted_foreground)
                .child(message)
        });
        let divider = || {
            div()
                .h(px(1.))
                .mx_neg_1p5()
                .bg(theme.foreground.opacity(0.08))
        };
        let panel = jig_commands::surface::panel(cx)
            .flex()
            .flex_col()
            .key_context("JigRunPicker")
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_up))
            .capture_action(cx.listener(Self::on_down))
            .w(px(WIDTH))
            .p_1p5()
            .child(
                h_flex()
                    .h(px(40.))
                    .pl_2p5()
                    .pr_1()
                    .gap_1()
                    .child(
                        Icon::default()
                            .data(PLAY)
                            .size(px(15.))
                            .flex_none()
                            .text_color(theme.success),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(16.))
                            .child(Input::new(&self.input).appearance(false).cleanable(false)),
                    ),
            )
            .child(divider().mb_1())
            .child(v_flex().children(empty).child(list))
            .child(divider().mt_1())
            .child(h_flex().px_2p5().pt_1p5().pb_0p5().justify_end().child(
                jig_commands::surface::hint("↑↓ navigate · ↩ run · esc close", cx),
            ));
        jig_commands::motion::pop_in(panel, "jig-run-picker")
    }
}
