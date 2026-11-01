//! Resuming, stepping and stopping, the title bar's buttons for them, and breakpoints sent as they change.

use super::*;

impl Workspace {
    /// A breakpoint changed in `path`: tell a running debugger.
    pub(in crate::workspace) fn breakpoints_changed(&mut self, path: &std::path::Path, cx: &App) {
        let lines = self.breakpoints_in(path, cx);
        let logpoints = self.live.logpoints(path, &lines);
        if let Some(debug) = self.debug_mut().filter(|d| d.configured && d.is_live()) {
            for id in 0..debug.clients.len() {
                debug.request_to(
                    id,
                    "setBreakpoints",
                    set_breakpoints_arguments(path, &lines, &logpoints),
                    Pending::Command("Setting breakpoints"),
                );
            }
        }
    }

    /// Stop debugging: ask the debugger to end the program.
    pub(in crate::workspace) fn stop_debugging(&mut self) -> bool {
        let Some(debug) = self
            .debug_mut()
            .filter(|d| !d.clients.is_empty() && d.is_live())
        else {
            return false;
        };
        // The session's own connection ends everything it started.
        debug.request_to(
            0,
            "disconnect",
            json!({"terminateDebuggee": true}),
            Pending::Command("Stopping"),
        );
        true
    }

    /// Resume or step the paused thread.
    fn control(&mut self, command: &'static str, cx: &mut Context<Self>) {
        let Some(debug) = self.debug_mut().filter(|d| d.phase == Phase::Paused) else {
            return;
        };
        let Some(thread) = debug.thread_id else {
            return;
        };
        debug.request(
            command,
            json!({"threadId": thread}),
            Pending::Command("Stepping"),
        );
        debug.forget_pause();
        debug.phase = Phase::Running;
        self.clear_execution_line(cx);
        cx.notify();
    }

    pub(in crate::workspace) fn resume(
        &mut self,
        _: &Resume,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.control("continue", cx);
    }

    pub(in crate::workspace) fn step_over(
        &mut self,
        _: &StepOver,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.control("next", cx);
    }

    pub(in crate::workspace) fn step_into(
        &mut self,
        _: &StepInto,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.control("stepIn", cx);
    }

    pub(in crate::workspace) fn step_out(
        &mut self,
        _: &StepOut,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.control("stepOut", cx);
    }

    /// Resume, step and view buttons for the panel's header.
    pub(in crate::workspace) fn render_debug_buttons(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(debug) = self.debug() else {
            return Vec::new();
        };
        let theme = cx.theme();
        let paused = debug.phase == Phase::Paused;
        // Labelled on hover with what they do and their shortcut.
        let button = |id: &'static str,
                      icon: &'static [u8],
                      color: Hsla,
                      enabled: bool,
                      label: &'static str,
                      action: &'static dyn Action| {
            div()
                .id(id)
                .tooltip(move |window, cx| {
                    Tooltip::new(label).action(action, None).build(window, cx)
                })
                .flex_none()
                .size(px(22.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .when(enabled, |this| {
                    this.hover(|s| s.bg(theme.foreground.opacity(0.08)))
                })
                .when(!enabled, |this| this.opacity(0.35))
                .child(Icon::default().data(icon).size(px(12.)).text_color(color))
        };
        let tint = theme.blue;
        let mut buttons = vec![
            button(
                "debug-resume",
                RESUME,
                theme.success,
                paused,
                "Continue",
                &Resume,
            )
            .on_click(cx.listener(|this, _, window, cx| this.resume(&Resume, window, cx)))
            .into_any_element(),
            button(
                "debug-step-over",
                STEP_OVER,
                tint,
                paused,
                "Step Over",
                &StepOver,
            )
            .on_click(cx.listener(|this, _, window, cx| this.step_over(&StepOver, window, cx)))
            .into_any_element(),
            button(
                "debug-step-into",
                STEP_INTO,
                tint,
                paused,
                "Step Into",
                &StepInto,
            )
            .on_click(cx.listener(|this, _, window, cx| this.step_into(&StepInto, window, cx)))
            .into_any_element(),
            button(
                "debug-step-out",
                STEP_OUT,
                tint,
                paused,
                "Step Out",
                &StepOut,
            )
            .on_click(cx.listener(|this, _, window, cx| this.step_out(&StepOut, window, cx)))
            .into_any_element(),
        ];
        let tab = |id: &'static str, label: &'static str, view: View| {
            let active = debug.view == view;
            div()
                .id(id)
                .px_2()
                .h(px(20.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .text_size(px(11.5))
                .when(active, |this| {
                    this.bg(theme.foreground.opacity(0.08))
                        .text_color(theme.foreground)
                })
                .when(!active, |this| {
                    this.text_color(theme.muted_foreground)
                        .hover(|s| s.text_color(theme.foreground))
                })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(debug) = this.debug_mut() {
                        debug.view = view;
                    }
                    cx.notify();
                }))
        };
        buttons.push(
            h_flex()
                .ml_2()
                .gap_0p5()
                .child(tab("debug-view-debugger", "Debugger", View::Debugger))
                .child(tab("debug-view-console", "Console", View::Console))
                .into_any_element(),
        );
        buttons
    }
}
