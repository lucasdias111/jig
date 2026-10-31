//! The strip along the bottom of the editor: the branch, with a switcher,
//! where the cursor is, and how the file is indented, encoded and
//! highlighted.

use gpui_kit::component::input::EditorState;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, h_flex};
use gpui_kit::*;

use super::Workspace;
use crate::{indentation, languages};
use jig_updates::{self as update, RestartToUpdate};

pub(super) const HEIGHT: f32 = 24.;

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
            .children(self.status_note.as_ref().map(|note| {
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme.warning)
                    .child(note.message.clone())
            }))
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
