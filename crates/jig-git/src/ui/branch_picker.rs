//! Switch branches: a Spotlight-style panel listing the repository's
//! branches, local ones first, filtered as you type. A name that matches no
//! branch can be created from where you are, and the first row, New
//! Branch…, turns the panel into a field for just that.

use std::ops::Range;

use gpui_kit::component::input::{Enter, Escape, Input, InputEvent, InputState, MoveDown, MoveUp};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::domain::status::Branch;
use crate::ui::panel::BRANCH_ICON;
use jig_commands::fuzzy;
use jig_commands::surface::PLUS_ICON as PLUS;

pub const WIDTH: f32 = 480.;
const ROW_HEIGHT: f32 = 32.;
const MAX_VISIBLE_ROWS: usize = 10;

pub enum BranchPickerEvent {
    Switch(Branch),
    Create(String),
    Dismissed,
    Blurred,
}

/// What the panel is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Switch,
    /// Only a name field for a new branch.
    Create,
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    /// Turns the panel to [`Mode::Create`].
    NewBranch,
    Branch {
        index: usize,
        /// Byte offsets of the name's characters the query matched.
        positions: Vec<usize>,
    },
    Create(String),
    /// A name already taken, shown so Enter doing nothing makes sense.
    Taken(String),
}

pub struct BranchPicker {
    mode: Mode,
    input: Entity<InputState>,
    /// The checked-out branch, which a new one starts from.
    head: Option<String>,
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
    pub fn new(
        mode: Mode,
        head: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder(mode)));
        let subscription = cx.subscribe_in(&input, window, |this, _, event, _, cx| match event {
            InputEvent::Change => this.refilter(cx),
            InputEvent::Blur => cx.emit(BranchPickerEvent::Blurred),
            _ => {}
        });
        input.update(cx, |input, cx| input.focus(window, cx));
        Self {
            mode,
            input,
            head,
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

    #[cfg(any(test, feature = "testing"))]
    pub fn row_names(&self) -> Vec<String> {
        self.rows
            .iter()
            .map(|row| match row {
                Row::Branch { index, .. } => self.branches[*index].name.clone(),
                Row::NewBranch => "New Branch…".into(),
                Row::Create(name) => format!("Create {name}"),
                Row::Taken(name) => format!("Taken {name}"),
            })
            .collect()
    }

    fn query(&self, cx: &App) -> String {
        self.input.read(cx).value().trim().to_string()
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx);
        if self.mode == Mode::Create {
            let name = branch_name(&query);
            self.rows = if name.is_empty() || self.loading {
                Vec::new()
            } else if self.branches.iter().any(|b| !b.remote && b.name == name) {
                vec![Row::Taken(name)]
            } else {
                vec![Row::Create(name)]
            };
            self.selected = 0;
            cx.notify();
            return;
        }
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
        if query.is_empty() && !self.loading {
            rows.insert(0, Row::NewBranch);
        } else if !query.is_empty() && !exists && !self.loading {
            rows.push(Row::Create(branch_name(&query)));
        }
        self.rows = rows;
        self.selected = 0;
        self.scroll.scroll_to_item(0, ScrollStrategy::Nearest);
        cx.notify();
    }

    /// Turn the panel into the new branch field, keeping what was typed.
    fn start_create(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.mode = Mode::Create;
        self.input.update(cx, |input, cx| {
            input.set_placeholder(placeholder(Mode::Create), window, cx)
        });
        self.refilter(cx);
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        match self.rows.get(index) {
            Some(Row::NewBranch) => self.start_create(window, cx),
            Some(Row::Taken(_)) | None => {}
            Some(Row::Branch { index, .. }) => {
                cx.emit(BranchPickerEvent::Switch(self.branches[*index].clone()))
            }
            Some(Row::Create(name)) => cx.emit(BranchPickerEvent::Create(name.clone())),
        }
    }

    fn on_enter(&mut self, _: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        cx.stop_propagation();
        self.choose(self.selected, window, cx);
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
            Row::NewBranch => (StyledText::new("New Branch…"), ""),
            Row::Create(name) => {
                let from = self
                    .head
                    .as_ref()
                    .map_or_else(String::new, |head| format!(" from {head}"));
                (
                    StyledText::new(SharedString::from(format!("Create branch “{name}”{from}"))),
                    "new",
                )
            }
            Row::Taken(name) => (
                StyledText::new(SharedString::from(format!("“{name}” already exists"))),
                "",
            ),
        };
        let icon = if self.rows[ix] == Row::NewBranch {
            PLUS
        } else {
            BRANCH_ICON
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
                    .data(icon)
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
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.choose(ix, window, cx);
                }),
            )
            .into_any_element()
    }
}

mod view;

fn placeholder(mode: Mode) -> &'static str {
    match mode {
        Mode::Switch => "Switch to branch…",
        Mode::Create => "New branch name…",
    }
}

/// What's typed as a branch name: spaces, which git refuses, become dashes.
fn branch_name(query: &str) -> String {
    query.trim().replace(' ', "-")
}
