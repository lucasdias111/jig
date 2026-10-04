//! Switch branches: a Spotlight-style panel listing the repository's
//! branches, local ones first, filtered as you type. A name that matches no
//! branch can be created from where you are.

use std::ops::Range;

use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::fuzzy;
use crate::git::Branch;
use crate::git_panel::BRANCH_ICON;

pub const WIDTH: f32 = 480.;
const ROW_HEIGHT: f32 = 32.;
const MAX_VISIBLE_ROWS: usize = 10;

pub enum BranchPickerEvent {
    Switch(Branch),
    Create(String),
    Dismissed,
    Blurred,
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Branch {
        index: usize,
        /// Byte offsets of the name's characters the query matched.
        positions: Vec<usize>,
    },
    Create(String),
}

pub struct BranchPicker {
    input: Entity<InputState>,
    branches: Vec<Branch>,
    /// Still asking git.
    loading: bool,
    rows: Vec<Row>,
    selected: usize,
    scroll: UniformListScrollHandle,
    _subscription: Subscription,
}

impl EventEmitter<BranchPickerEvent> for BranchPicker {}

impl Focusable for BranchPicker {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl BranchPicker {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder("Switch to branch…"));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, _, cx| match event {
            InputEvent::Change => this.refilter(cx),
            InputEvent::Blur => cx.emit(BranchPickerEvent::Blurred),
            _ => {}
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        Self {
            input,
            branches: Vec::new(),
            loading: true,
            rows: Vec::new(),
            selected: 0,
            scroll: UniformListScrollHandle::default(),
            _subscription: subscription,
        }
    }

    pub fn set_branches(&mut self, branches: Vec<Branch>, cx: &mut Context<Self>) {
        self.branches = branches;
        self.loading = false;
        self.refilter(cx);
    }

    #[cfg(test)]
    pub fn row_names(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| match row {
                Row::Branch { index, .. } => self.branches[*index].name.clone(),
                Row::Create(name) => format!("Create {name}"),
            })
            .collect()
    }

    fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().trim().to_string()
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let mut rows: Vec<Row> = if query.is_empty() {
            (0..self.branches.len())
                .map(|index| Row::Branch {
                    index,
                    positions: Vec::new(),
                })
                .collect()
        } else {
            fuzzy::match_paths(
                self.branches.iter().map(|b| b.name.as_str()),
                &query,
                usize::MAX,
            )
            .into_iter()
            .map(|m| Row::Branch {
                index: m.index,
                positions: m.positions,
            })
            .collect()
        };
        let exists = self.branches.iter().any(|b| b.name == query);
        if !query.is_empty() && !exists && !self.loading {
            rows.push(Row::Create(query.replace(' ', "-")));
        }
        self.rows = rows;
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Nearest);
        cx.notify();
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        match self.rows.get(index) {
            Some(Row::Branch { index, .. }) => {
                cx.emit(BranchPickerEvent::Switch(self.branches[*index].clone()))
            }
            Some(Row::Create(name)) => cx.emit(BranchPickerEvent::Create(name.clone())),
            None => {}
        }
    }

    fn on_enter(&mut self, _: &Enter, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.choose(self.selected, cx);
    }

    fn on_escape(&mut self, _: &Escape, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        cx.emit(BranchPickerEvent::Dismissed);
    }

    fn on_up(&mut self, _: &MoveUp, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if !self.rows.is_empty() {
            let len = self.rows.len();
            self.select((self.selected + len - 1) % len, cx);
        }
    }

    fn on_down(&mut self, _: &MoveDown, _: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        if !self.rows.is_empty() {
            self.select((self.selected + 1) % self.rows.len(), cx);
        }
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
        let (name, tag) = match &self.rows[ix] {
            Row::Branch { index, positions } => {
                let branch = &self.branches[*index];
                let mark = HighlightStyle {
                    color: (!selected).then_some(theme.primary),
                    font_weight: Some(FontWeight::BOLD),
                    ..Default::default()
                };
                let highlights: Vec<(Range<usize>, HighlightStyle)> = positions
                    .iter()
                    .map(|&i| {
                        let len = branch.name[i..].chars().next().map_or(1, char::len_utf8);
                        (i..i + len, mark)
                    })
                    .collect();
                let tag = if branch.current {
                    "current"
                } else if branch.remote {
                    "remote"
                } else {
                    ""
                };
                (
                    StyledText::new(SharedString::from(branch.name.clone()))
                        .with_highlights(highlights),
                    tag,
                )
            }
            Row::Create(name) => (
                StyledText::new(SharedString::from(format!("Create branch “{name}”"))),
                "new",
            ),
        };
        h_flex()
            .id(("branch-picker-row", ix))
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
                    .data(BRANCH_ICON)
                    .size(px(13.))
                    .flex_none()
                    .text_color(if selected { text } else { muted }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.5))
                    .child(name),
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

impl Render for BranchPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.rows.len();
        let list = uniform_list(
            "branch-picker-rows",
            rows,
            cx.processor(|this, range: Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .w_full()
        .h(px(rows.min(MAX_VISIBLE_ROWS) as f32 * ROW_HEIGHT));
        let empty = (rows == 0).then(|| {
            div()
                .px_2p5()
                .py_3()
                .text_size(px(13.))
                .text_color(theme.muted_foreground)
                .child(if self.loading {
                    "Reading branches…"
                } else {
                    "No branches yet. Type a name to create one."
                })
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
            .key_context("JigBranchPicker")
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
                            .data(BRANCH_ICON)
                            .size(px(15.))
                            .flex_none()
                            .text_color(theme.primary),
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
                jig_commands::surface::hint("↑↓ navigate · ↩ switch · esc close", cx),
            ));
        jig_commands::motion::pop_in(panel, "jig-branch-picker")
    }
}
