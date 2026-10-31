//! The strip along the bottom of the editor: the branch, with a switcher,
//! where the cursor is, and how the file is indented, encoded and
//! highlighted.

use std::time::Duration;

use gpui_kit::component::input::EditorState;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::Workspace;
use jig_kit::indentation;
use jig_languages as languages;
use jig_updates::{self as update, RestartToUpdate};

pub(super) const HEIGHT: f32 = 24.;
/// How long a message stays in the status bar.
const STATUS_TIMEOUT: Duration = Duration::from_secs(8);

/// A message in the status bar, such as why a save wasn't formatted, gone
/// after a while. A view of its own, so it clears itself.
#[derive(Default)]
pub(super) struct StatusMessage {
    showing: Option<Showing>,
    /// Which message is showing, so an older one's timer leaves a newer
    /// one be.
    serial: u64,
}

struct Showing {
    message: SharedString,
    _task: Task<()>,
}

impl StatusMessage {
    /// The message showing, if any.
    pub(super) fn message(&self) -> Option<&SharedString> {
        self.showing.as_ref().map(|showing| &showing.message)
    }

    /// Show `message` for a while, in place of any other.
    fn show(&mut self, message: SharedString, cx: &mut Context<Self>) {
        self.serial += 1;
        let serial = self.serial;
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(STATUS_TIMEOUT).await;
            this.update(cx, |this, cx| {
                if this.serial == serial {
                    this.showing = None;
                    cx.notify();
                }
            })
            .ok();
        });
        self.showing = Some(Showing {
            message,
            _task: task,
        });
        cx.notify();
    }
}

impl Render for StatusMessage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .min_w_0()
            .truncate()
            .text_color(cx.theme().warning)
            .children(self.message().cloned())
    }
}

/// `Ln 12, Col 5`, for one tab's editor. A view of its own so the cursor
/// moving repaints just this, not the whole workspace.
pub(super) struct CursorPosition {
    editor: Entity<EditorState>,
    _observe: Subscription,
}

impl CursorPosition {
    pub(super) fn new(editor: Entity<EditorState>, cx: &mut Context<Self>) -> Self {
        let observe = cx.observe(&editor, |_, _, cx| cx.notify());
        Self {
            editor,
            _observe: observe,
        }
    }
}

impl Render for CursorPosition {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.editor.read(cx);
        let position = state.cursor_position();
        let selection = state.selected_range();
        let mut label = format!("Ln {}, Col {}", position.line + 1, position.character + 1);
        if !selection.is_empty() {
            let text = state.text();
            let selected =
                text.byte_to_char_idx(selection.end) - text.byte_to_char_idx(selection.start);
            label.push_str(&format!(" ({selected} selected)"));
        }
        div().child(label)
    }
}

impl Workspace {
    /// Show `message` in the status bar for a while.
    pub(super) fn show_status(&mut self, message: SharedString, cx: &mut Context<Self>) {
        self.status_note
            .update(cx, |status, cx| status.show(message, cx));
    }

    pub(super) fn render_status_bar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tab = self.tab();
        let language = tab
            .document
            .path
            .as_deref()
            .and_then(|path| languages::written_in(path, &self.settings.languages))
            .map_or("Plain Text", |language| language.label);
        let item = |text: SharedString| div().child(text);
        h_flex()
            .flex_none()
            .h(px(HEIGHT))
            .pl_1p5()
            .pr_3()
            .gap_4()
            .border_t_1()
            .border_color(theme.title_bar_border)
            .text_xs()
            .text_color(theme.muted_foreground)
            .children(self.render_branch_switcher(cx))
            .children(self.render_problem_counts(cx))
            .when(self.status_note.read(cx).message().is_some(), |this| {
                this.child(self.status_note.clone())
            })
            .child(div().flex_1())
            .children(render_update(cx))
            .child(tab.cursor.clone())
            .child(item(indentation::label(tab.indentation).into()))
            .child(item(tab.document.encoding().into()))
            .child(item(tab.document.line_ending().into()))
            .child(item(language.into()))
    }
}

/// "Restart to Update" once an update is downloaded, or where to get one
/// this copy of Jig can't install itself.
fn render_update(cx: &App) -> Option<AnyElement> {
    let theme = cx.theme();
    let (label, tooltip): (SharedString, SharedString) = match update::state(cx) {
        update::State::Ready { version, .. } => (
            "Restart to Update".into(),
            format!("Jig {version} is downloaded").into(),
        ),
        update::State::Manual { version, reason } => {
            (format!("Jig {version} Available").into(), reason.into())
        }
        _ => return None,
    };
    let ready = matches!(update::state(cx), update::State::Ready { .. });
    let accent = theme.primary;
    Some(
        h_flex()
            .id("update")
            .debug_selector(|| "update".into())
            .flex_none()
            .h_full()
            .px_1p5()
            .gap_1()
            .text_color(accent)
            .hover(move |s| s.bg(accent.opacity(0.12)))
            .child(Icon::new(IconName::ArrowDown).size(px(12.)))
            .child(label)
            .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
            .on_click(move |_, window, cx| {
                if ready {
                    window.dispatch_action(Box::new(RestartToUpdate), cx);
                } else {
                    update::open_releases_page(cx);
                }
            })
            .into_any_element(),
    )
}
