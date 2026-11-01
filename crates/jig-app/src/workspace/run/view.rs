//! The title bar's run controls and the output panel.

use super::*;

impl Workspace {
    pub(in crate::workspace) fn toggle_run_panel(
        &mut self,
        _: &ToggleRunPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.runs.session.is_none() {
            self.open_run_picker(window, cx);
            return;
        }
        self.runs.panel_open = !self.runs.panel_open;
        cx.notify();
    }

    #[cfg(test)]
    pub(in crate::workspace) fn run_picker_rows(&self, cx: &App) -> Option<Vec<String>> {
        self.run_configs.read(cx).picker_rows(cx)
    }

    /// The current run's id, output and ending.
    #[cfg(test)]
    pub(in crate::workspace) fn run_output(
        &self,
        cx: &App,
    ) -> Option<(u64, Vec<OutputLine>, Option<String>)> {
        let session = self.runs.session.as_ref()?;
        let lines = self.run_output.read(cx).lines().to_vec();
        Some((session.id, lines, session.ending.clone()))
    }

    /// The picker, while it's open.
    pub(in crate::workspace) fn render_run_picker(&self, cx: &App) -> Option<AnyElement> {
        self.run_configs
            .read(cx)
            .picker_open()
            .then(|| self.run_configs.clone().into_any_element())
    }

    /// The configuration ⌃R runs, and buttons to run and stop it, at the
    /// right of the title bar. Only with a project to run.
    pub(in crate::workspace) fn render_run_controls(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        self.run_root(cx)?;
        let theme = cx.theme();
        let running = self
            .runs
            .session
            .as_ref()
            .is_some_and(RunSession::is_running);
        let name = self
            .run_configs
            .read(cx)
            .selected()
            .map_or("Run…".to_string(), |config| config.name.clone());
        let button = |id: &'static str, icon: &'static [u8], color: Hsla| {
            div()
                .id(id)
                .flex_none()
                .size(px(26.))
                .rounded(px(6.))
                .flex()
                .items_center()
                .justify_center()
                .debug_selector(move || id.into())
                .hover(|s| s.bg(theme.foreground.opacity(0.08)))
                // Otherwise the title bar takes the press as a window drag.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(Icon::default().data(icon).size(px(13.)).text_color(color))
        };
        Some(
            h_flex()
                .flex_none()
                .ml_2()
                .gap_0p5()
                .child(
                    h_flex()
                        .id("run-configuration")
                        .flex_none()
                        .h(px(26.))
                        .px_2()
                        .gap_1p5()
                        .rounded(px(6.))
                        .text_size(px(12.5))
                        .text_color(theme.foreground.opacity(0.85))
                        .hover(|s| s.bg(theme.foreground.opacity(0.08)))
                        .when(running, |this| {
                            this.child(
                                div()
                                    .flex_none()
                                    .size(px(6.))
                                    .rounded_full()
                                    .bg(theme.success),
                            )
                        })
                        // Never cut: the title or tabs give way instead.
                        .child(div().flex_none().whitespace_nowrap().child(name))
                        .child(
                            Icon::default()
                                .data(CHEVRON)
                                .size(px(10.))
                                .flex_none()
                                .text_color(theme.muted_foreground),
                        )
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.open_run_picker(window, cx)),
                        ),
                )
                .child(
                    button(
                        "run-selected",
                        if running { RERUN } else { PLAY },
                        theme.success,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.run_selected(&RunSelected, window, cx)
                    })),
                )
                .child(
                    button("debug-selected", super::super::debug::BUG, theme.success).on_click(
                        cx.listener(|this, _, window, cx| {
                            this.debug_selected(&super::super::DebugSelected, window, cx)
                        }),
                    ),
                )
                .when(running, |this| {
                    this.child(button("stop-run", STOP, theme.danger).on_click(
                        cx.listener(|this, _, window, cx| this.stop_run(&StopRun, window, cx)),
                    ))
                })
                .into_any_element(),
        )
    }

    /// The output panel under the editor, while it's open.
    pub(in crate::workspace) fn render_run_panel(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let session = self
            .runs
            .session
            .as_ref()
            .filter(|_| self.runs.panel_open)?;
        let theme = cx.theme();
        let running = session.is_running();
        let debug_status = session.debug.as_ref().and_then(|debug| debug.status());
        let paused = session
            .debug
            .as_ref()
            .is_some_and(|debug| debug.phase == super::super::debug::Phase::Paused);
        let status = match (&session.ending, debug_status) {
            (Some(ending), _) => ending.clone(),
            (None, Some(status)) => status,
            (None, None) => "Running…".into(),
        };
        let status_color = match &session.ending {
            None if paused => theme.blue,
            None => theme.success,
            Some(ending) if ending.ends_with("exit code 0") => theme.muted_foreground,
            Some(_) => theme.danger,
        };
        let button = |id: &'static str, icon: &'static [u8], color: Hsla| {
            div()
                .id(id)
                .flex_none()
                .size(px(22.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .hover(|s| s.bg(theme.foreground.opacity(0.08)))
                .child(Icon::default().data(icon).size(px(12.)).text_color(color))
        };
        let header = h_flex()
            .flex_none()
            .h(px(HEADER_HEIGHT))
            .pl_3()
            .pr_2()
            .gap_2()
            .text_size(px(12.))
            .child(
                div()
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground.opacity(0.85))
                    .child(session.config.name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(status_color)
                    .child(status),
            )
            .children(self.render_debug_buttons(cx))
            .child(
                button("panel-rerun", RERUN, theme.success).on_click(cx.listener(
                    |this, _, window, cx| {
                        let Some(session) = this.runs.session.as_ref() else {
                            return;
                        };
                        let config = session.config.clone();
                        if session.debug.is_some() {
                            this.start_debug(config, window, cx);
                        } else {
                            this.start_run(config, window, cx);
                        }
                    },
                )),
            )
            .when(running, |this| {
                this.child(
                    button("panel-stop", STOP, theme.danger)
                        .on_click(|_, window, cx| window.dispatch_action(Box::new(StopRun), cx)),
                )
            })
            .child(
                button("panel-hide", CLOSE, theme.muted_foreground).on_click(cx.listener(
                    |this, _, window, cx| {
                        this.runs.panel_open = false;
                        this.focus_main(window, cx);
                        cx.notify();
                    },
                )),
            );

        Some(
            v_flex()
                .relative()
                .flex_none()
                .h(self.runs.panel_height)
                .border_t_1()
                .border_color(theme.title_bar_border)
                .child(header)
                .map(|this| match self.render_debugger(cx) {
                    Some(debugger) => this.child(debugger),
                    None => this.child(self.run_output.clone()),
                })
                .child(
                    // The draggable top edge.
                    div()
                        .id("run-panel-resize")
                        .absolute()
                        .top(px(-3.))
                        .left_0()
                        .right_0()
                        .h(px(6.))
                        .cursor_row_resize()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.runs.resizing = true;
                                cx.stop_propagation();
                            }),
                        ),
                )
                .into_any_element(),
        )
    }

    /// Mouse handlers for the whole window, active while the panel's top
    /// edge is being dragged.
    pub(in crate::workspace) fn run_panel_drag_handlers(
        &self,
        root: Div,
        cx: &Context<Self>,
    ) -> Div {
        let below = if self.home {
            0.
        } else {
            super::super::status_bar::HEIGHT
        };
        root.when(self.runs.resizing, |root| {
            root.cursor_row_resize()
                .on_mouse_move(
                    cx.listener(move |this, event: &MouseMoveEvent, window, cx| {
                        if !event.dragging() {
                            this.runs.resizing = false;
                        } else {
                            let bottom = window.viewport_size().height - px(below);
                            let max = (bottom - px(TITLE_BAR_HEIGHT + MIN_EDITOR_HEIGHT))
                                .max(px(MIN_PANEL_HEIGHT));
                            this.runs.panel_height =
                                (bottom - event.position.y).clamp(px(MIN_PANEL_HEIGHT), max);
                        }
                        cx.notify();
                    }),
                )
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.runs.resizing = false;
                        cx.notify();
                    }),
                )
        })
    }
}
