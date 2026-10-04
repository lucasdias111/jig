//! A small button just right of the selection that opens the command input,
//! so commands are in reach where the eyes already are, without knowing ⌘K.

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::*;
use jig_editor::EditorHandle;

use super::{OpenCommand, Workspace};

/// Room between the end of the selection's longest line and the button.
const GAP: f32 = 8.;
const SIZE: f32 = 22.;

/// A view of its own, like the status bar's cursor position: the selection
/// moving repaints just this, not the whole workspace.
pub(super) struct SelectionButton {
    workspace: WeakEntity<Workspace>,
    _workspace_changed: Subscription,
    _settings_changed: Subscription,
    /// The editor being followed, which changes with the active tab.
    following: Option<(EntityId, Subscription)>,
}

impl SelectionButton {
    pub(super) fn new(workspace: &Entity<Workspace>, cx: &mut Context<Self>) -> Self {
        Self {
            workspace: workspace.downgrade(),
            _workspace_changed: cx.observe(workspace, |_, _, cx| cx.notify()),
            _settings_changed: cx
                .observe_global::<crate::settings::AppSettings>(|_, cx| cx.notify()),
            following: None,
        }
    }

    /// Where the button goes, or `None` when it shouldn't show: turned off
    /// in Settings, nothing selected, the editor not focused, the selection
    /// off screen, or a command, preview or panel already on screen.
    fn position(workspace: &Workspace, window: &Window, cx: &App) -> Option<Point<Pixels>> {
        if !crate::settings::get(cx).editor.selection_button
            || workspace.home
            || workspace.modal_open()
            || workspace.run.is_some()
        {
            return None;
        }
        let editor = workspace.editor();
        let selection = editor.selection(cx);
        if selection.is_empty() || !editor.state().focus_handle(cx).is_focused(window) {
            return None;
        }
        let beside = editor.beside_point(selection, cx)?;
        // Centred on the first line, whose height is about the button's.
        Some(beside + point(px(GAP), px(-2.)))
    }
}

impl Render for SelectionButton {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(workspace) = self.workspace.upgrade() else {
            return div().into_any_element();
        };
        // Follow the active tab's editor, so selecting repaints the button.
        let state = workspace.read(cx).editor().state().clone();
        if self.following.as_ref().map(|(id, _)| *id) != Some(state.entity_id()) {
            self.following = Some((
                state.entity_id(),
                cx.observe(&state, |_, _, cx| cx.notify()),
            ));
        }
        let Some(position) = Self::position(workspace.read(cx), window, cx) else {
            return div().into_any_element();
        };

        let theme = cx.theme();
        let accent = jig_commands::surface::lane_accent(false, cx);
        let button = div()
            .id("selection-command")
            .debug_selector(|| "selection-command".into())
            .occlude()
            .size(px(SIZE))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(6.))
            .bg(theme.popover)
            .border_1()
            .border_color(theme.foreground.opacity(0.1))
            .shadow_sm()
            .cursor_pointer()
            .hover(|this| this.bg(accent.opacity(0.14)))
            .tooltip(|window, cx| Tooltip::new("Commands for the selection").build(window, cx))
            .child(
                Icon::default()
                    .data(super::commands::COMMAND_ICON)
                    .size(px(13.))
                    .text_color(accent),
            )
            // Keep the press from reaching the code, which would collapse the
            // selection the commands are for.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| {
                if let Some(workspace) = this.workspace.upgrade() {
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_command(&OpenCommand, window, cx)
                    });
                }
            }));
        deferred(anchored().position(position).child(button))
            .priority(1)
            .into_any_element()
    }
}
