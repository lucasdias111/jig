//! Run configurations: choosing one (⌃⌥R), running it (⌃R), stopping it
//! (⌘F2), and the panel under the editor that shows its output (⌘J).
//!
//! One configuration runs at a time; running another, or the same again,
//! stops the one before. Unsaved files are saved first, as IntelliJ does,
//! so what runs is what's on screen.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::channel::mpsc::{TryRecvError, UnboundedReceiver};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_editor::EditorHandle as _;

use super::{
    ChooseRunConfiguration, EditRunConfigurations, RunSelected, StopRun, TITLE_BAR_HEIGHT,
    ToggleRunPanel, Workspace,
};
use jig_run::run_configs::{self, RunConfig};
use jig_run::run_output::{Link, OutputLine, Process, RunEvent};
use jig_run::run_picker::PLAY;

pub(super) mod configurations;
pub(super) mod output;

use configurations::{RunConfigurations, RunConfigurationsEvent};
use output::{RunOutput, RunOutputEvent};

const HEADER_HEIGHT: f32 = 30.;
const MIN_PANEL_HEIGHT: f32 = 90.;
/// Room the panel leaves the editor above it, at least.
const MIN_EDITOR_HEIGHT: f32 = 160.;
/// Events taken from the process per update, so a flood still repaints.
const MAX_BATCH: usize = 2_000;
/// How often new output is looked for.
const POLL_INTERVAL: Duration = Duration::from_millis(30);

const STOP: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="black"><rect x="5.5" y="5.5" width="13" height="13" rx="2"/></svg>"#;
const RERUN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 11a8 8 0 1 0-2.3 5.7"/><path d="M20 4v7h-7"/></svg>"#;
const CLOSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"#;
const CHEVRON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;

pub(super) struct RunState {
    pub(super) session: Option<RunSession>,
    pub(super) panel_open: bool,
    panel_height: Pixels,
    /// Set while the panel's top edge is being dragged.
    resizing: bool,
    next_id: u64,
}

impl RunState {
    pub(super) fn new() -> Self {
        Self {
            session: None,
            panel_open: false,
            panel_height: px(220.),
            resizing: false,
            next_id: 0,
        }
    }
}

/// One run of a configuration. What it printed is in the window's
/// [`RunOutput`]. Debugging is a run with a debugger attached.
pub(super) struct RunSession {
    id: u64,
    pub(super) config: RunConfig,
    /// The program, or for a debug session, its build.
    pub(super) process: Option<Process>,
    events: UnboundedReceiver<RunEvent>,
    /// How it ended, once it has.
    pub(super) ending: Option<String>,
    pub(super) started: Instant,
    pub(super) debug: Option<super::debug::DebugSession>,
    _poll: Task<()>,
}

impl RunSession {
    fn is_running(&self) -> bool {
        self.ending.is_none()
            && (self.process.as_ref().is_some_and(Process::is_running)
                || self.debug.as_ref().is_some_and(|debug| debug.is_live()))
    }
}

impl Workspace {
    /// The project configurations belong to: the sidebar's, or the open
    /// file's.
    pub(super) fn run_root(&self, cx: &App) -> Option<PathBuf> {
        self.project_root(cx).or_else(|| {
            self.document()
                .path
                .as_deref()
                .map(jig_kit::project::root_for)
        })
    }

    pub(super) fn run_selected(
        &mut self,
        _: &RunSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.run_configs.read(cx).selected().cloned() {
            Some(config) => self.start_run(config, window, cx),
            None => self.open_run_picker(window, cx),
        }
    }

    pub(super) fn choose_run_configuration(
        &mut self,
        _: &ChooseRunConfiguration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_run_picker(window, cx);
    }

    pub(super) fn stop_run(&mut self, _: &StopRun, _: &mut Window, cx: &mut Context<Self>) {
        self.stop_process();
        cx.notify();
    }

    pub(super) fn stop_process(&mut self) {
        if self.stop_debugging() {
            return;
        }
        if let Some(process) = self
            .runs
            .session
            .as_ref()
            .and_then(|session| session.process.as_ref())
        {
            process.stop();
        }
    }

    pub(super) fn toggle_run_panel(
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

    pub(super) fn edit_run_configurations(
        &mut self,
        _: &EditRunConfigurations,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_run_file(window, cx);
    }

    /// Open `.jig/run.toml`, starting it from the template if there is none.
    fn open_run_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(root) = self.run_root(cx) else {
            self.show_error(
                "Open a folder first, so Jig knows which project to run.",
                window,
                cx,
            );
            return;
        };
        let path = root.join(run_configs::FILE);
        if !path.exists() {
            let created = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, run_configs::TEMPLATE));
            if let Err(error) = created {
                self.show_error(
                    &format!("Couldn't create {}: {error}", run_configs::FILE),
                    window,
                    cx,
                );
                return;
            }
            self.refresh_tree(cx);
        }
        self.open_file(&path, window, cx);
    }

    /// After a save: a changed `run.toml` takes effect at once.
    pub(super) fn run_file_saved(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(root) = self.run_root(cx) else {
            return;
        };
        if super::tabs::canonical(path) == super::tabs::canonical(&root.join(run_configs::FILE)) {
            self.run_configs
                .update(cx, |configurations, cx| configurations.load(root, cx));
        }
    }

    fn open_run_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_run_picker_for(false, window, cx);
    }

    /// The picker, to run what's chosen or with `debug`, to debug it.
    pub(super) fn open_run_picker_for(
        &mut self,
        debug: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.run_configs.read(cx).picker_open() {
            return;
        }
        let Some(root) = self.run_root(cx) else {
            self.show_error(
                "Open a folder first, so Jig knows which project to run.",
                window,
                cx,
            );
            return;
        };
        self.palette = None;
        self.close_quick_open(cx);
        self.close_find_in_files(cx);
        self.run_configs.update(cx, |configurations, cx| {
            configurations.open_picker(root, debug, window, cx)
        });
        cx.notify();
    }

    /// Act on what the configurations say: run or debug what was chosen,
    /// edit `run.toml`, or back to where the keyboard was.
    pub(super) fn on_run_configurations_event(
        &mut self,
        _: &Entity<RunConfigurations>,
        event: &RunConfigurationsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            RunConfigurationsEvent::Chosen { config, debug } => {
                self.focus_main(window, cx);
                cx.notify();
                if *debug {
                    self.start_debug(config.clone(), window, cx);
                } else {
                    self.start_run(config.clone(), window, cx);
                }
            }
            RunConfigurationsEvent::Edit => {
                cx.notify();
                self.open_run_file(window, cx);
            }
            RunConfigurationsEvent::Closed { refocus } => {
                if *refocus {
                    self.focus_main(window, cx);
                }
                cx.notify();
            }
            RunConfigurationsEvent::LoadFailed(error) => self.show_error(error, window, cx),
        }
    }

    pub(super) fn start_run(
        &mut self,
        config: RunConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_session(config.clone(), Some(config), false, None, window, cx);
    }

    /// Start a session for `config`, running `process` (the program, or a
    /// debug session's build) if given. With `json_stdout`, the process's
    /// JSON lines are read as data.
    pub(super) fn start_session(
        &mut self,
        config: RunConfig,
        process: Option<RunConfig>,
        json_stdout: bool,
        debug: Option<super::debug::DebugSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_all_for_run(window, cx);
        // Stopped by being dropped.
        self.runs.session = None;
        self.clear_execution_line(cx);
        self.runs.next_id += 1;
        let id = self.runs.next_id;
        let root = self.run_root(cx).unwrap_or_else(|| config.cwd.clone());

        let (sender, events) = futures::channel::mpsc::unbounded();
        let mut lines = Vec::new();
        let mut ending = None;
        let process = process.and_then(|process| {
            lines.push(OutputLine::meta(format!("$ {}", process.command)));
            match Process::spawn(&process, &root, json_stdout, sender) {
                Ok(process) => Some(process),
                Err(error) => {
                    let message = format!("{error:#}");
                    lines.push(OutputLine::meta(message.clone()));
                    ending = Some(message);
                    None
                }
            }
        });
        let mut debug = debug;
        if ending.is_some()
            && let Some(debug) = debug.as_mut()
        {
            debug.phase = super::debug::Phase::Ended;
        }
        // Polled rather than awaited: output comes from the process's and
        // the debugger's own threads, and a timer batches a flood of it
        // into fewer repaints.
        let poll = cx.spawn_in(window, async move |this, cx| {
            loop {
                let busy = this.update_in(cx, |this, window, cx| this.poll_session(id, window, cx));
                match busy {
                    Ok(Some(true)) => {}
                    Ok(Some(false)) => cx.background_executor().timer(POLL_INTERVAL).await,
                    Ok(None) | Err(_) => break,
                }
            }
        });
        self.run_output
            .update(cx, |output, cx| output.start(lines, cx));
        self.runs.session = Some(RunSession {
            id,
            config,
            process,
            events,
            ending,
            started: Instant::now(),
            debug,
            _poll: poll,
        });
        self.runs.panel_open = true;
        cx.notify();
    }

    /// Take in what the process and the debugger said. `None` once session
    /// `id` is over; `Some(true)` when there was more than one batch's worth.
    fn poll_session(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        let session = self.runs.session.as_mut().filter(|s| s.id == id)?;
        let mut events = Vec::new();
        while events.len() < MAX_BATCH {
            match session.events.try_recv() {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            }
        }
        let full = events.len() == MAX_BATCH;
        let follow = self.run_output.read(cx).at_end();
        let before = self.run_output.read(cx).len();
        for event in events {
            self.on_run_event(event, window, cx);
        }
        let messages = self.take_dap_messages();
        let any = !messages.is_empty();
        for (id, message) in messages {
            self.on_dap_message(id, message, window, cx);
        }
        let session = self.runs.session.as_mut().filter(|s| s.id == id)?;
        self.run_output.update(cx, |output, cx| output.trim(cx));
        let output = self.run_output.read(cx);
        if output.len() != before || any {
            if follow {
                output.scroll_to_end();
            }
            cx.notify();
        }
        // Done when it has ended and nothing is left to read.
        let live = session.ending.is_none() || session.process.is_some();
        live.then_some(full)
    }

    fn on_run_event(&mut self, event: RunEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.runs.session.as_mut() else {
            return;
        };
        match event {
            RunEvent::Line(line) => self.run_output.update(cx, |o, cx| o.push(line, cx)),
            RunEvent::Data(json) => self.on_build_data(&json),
            RunEvent::Exited { message, success } => {
                if session.debug.is_some() {
                    let stopped = message == "Process stopped";
                    if success {
                        self.run_output
                            .update(cx, |o, cx| o.push(OutputLine::meta("Build finished"), cx));
                    } else if !stopped {
                        self.run_output
                            .update(cx, |o, cx| o.push(OutputLine::meta(message), cx));
                    }
                    self.on_build_exit(success, stopped, window, cx);
                    return;
                }
                session.process = None;
                let elapsed = session.started.elapsed().as_secs_f32();
                let line = OutputLine::meta(format!("{message} ({elapsed:.1}s)"));
                self.run_output.update(cx, |o, cx| o.push(line, cx));
                session.ending = Some(message);
            }
        }
    }

    /// Save every changed file that has somewhere to go.
    fn save_all_for_run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut saved_active = false;
        let mut failed = Vec::new();
        for ix in 0..self.tabs.len() {
            let tab = &self.tabs[ix];
            let Some(path) = tab.document.path.clone().filter(|_| tab.dirty) else {
                continue;
            };
            let text = tab.editor.text(cx);
            match self.tabs[ix].document.save(&path, &text) {
                Ok(()) => {
                    self.tabs[ix].dirty = false;
                    saved_active |= ix == self.active;
                }
                Err(_) => failed.push(self.tabs[ix].document.title()),
            }
        }
        if saved_active {
            self.lsp_saved(cx);
        }
        self.update_title(window);
        if !failed.is_empty() {
            self.show_error(
                &format!("Couldn't save {}; running anyway.", failed.join(", ")),
                window,
                cx,
            );
        }
    }

    /// A link in the output was clicked: open the code there.
    pub(super) fn on_run_output_event(
        &mut self,
        _: &Entity<RunOutput>,
        event: &RunOutputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let RunOutputEvent::OpenLink(link) = event;
        self.open_link(link, window, cx);
    }

    pub(super) fn open_link(&mut self, link: &Link, window: &mut Window, cx: &mut Context<Self>) {
        let line = link.line.saturating_sub(1);
        let column = link.column.unwrap_or(1).saturating_sub(1);
        let position = lsp_types::Position::new(line, column);
        self.open_at(
            &link.path,
            lsp_types::Range::new(position, position),
            window,
            cx,
        );
    }

    #[cfg(test)]
    pub(super) fn run_picker_rows(&self, cx: &App) -> Option<Vec<String>> {
        self.run_configs.read(cx).picker_rows(cx)
    }

    /// The current run's id, output and ending.
    #[cfg(test)]
    pub(super) fn run_output(&self, cx: &App) -> Option<(u64, Vec<OutputLine>, Option<String>)> {
        let session = self.runs.session.as_ref()?;
        let lines = self.run_output.read(cx).lines().to_vec();
        Some((session.id, lines, session.ending.clone()))
    }

    /// The picker, while it's open.
    pub(super) fn render_run_picker(&self, cx: &App) -> Option<AnyElement> {
        self.run_configs
            .read(cx)
            .picker_open()
            .then(|| self.run_configs.clone().into_any_element())
    }

    /// The configuration ⌃R runs, and buttons to run and stop it, at the
    /// right of the title bar. Only with a project to run.
    pub(super) fn render_run_controls(&self, cx: &Context<Self>) -> Option<AnyElement> {
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
                    button("debug-selected", super::debug::BUG, theme.success).on_click(
                        cx.listener(|this, _, window, cx| {
                            this.debug_selected(&super::DebugSelected, window, cx)
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
    pub(super) fn render_run_panel(&self, cx: &Context<Self>) -> Option<AnyElement> {
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
            .is_some_and(|debug| debug.phase == super::debug::Phase::Paused);
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
    pub(super) fn run_panel_drag_handlers(&self, root: Div, cx: &Context<Self>) -> Div {
        let below = if self.home {
            0.
        } else {
            super::status_bar::HEIGHT
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
