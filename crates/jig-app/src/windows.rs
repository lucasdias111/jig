//! Which of Jig's windows are editor windows, so Settings and the updater
//! can send an action to one without knowing what the other windows are.

use gpui_kit::*;

/// Says whether a window is an editor window; the app sets it as it starts.
struct EditorWindows(fn(AnyWindowHandle, &App) -> bool);

impl Global for EditorWindows {}

pub fn init(cx: &mut App, is_editor_window: fn(AnyWindowHandle, &App) -> bool) {
    cx.set_global(EditorWindows(is_editor_window));
}

/// Run `action` in the frontmost editor window, bringing it forward.
pub fn send_to_editor(action: Box<dyn Action>, cx: &mut App) {
    cx.defer(move |cx| {
        let Some(&EditorWindows(is_editor_window)) = cx.try_global::<EditorWindows>() else {
            return;
        };
        let windows = cx.window_stack().unwrap_or_else(|| cx.windows());
        let Some(target) = windows
            .into_iter()
            .find(|window| is_editor_window(*window, cx))
        else {
            return;
        };
        target
            .update(cx, |_, window, cx| {
                window.activate_window();
                window.dispatch_action(action, cx);
            })
            .ok();
        cx.activate(true);
    });
}
