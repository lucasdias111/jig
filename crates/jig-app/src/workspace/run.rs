//! Run configurations: choosing one (⌃⌥R), running it (⌃R), stopping it
//! (⌘F2), and the panel under the editor that shows its output (⌘J).
//!
//! One configuration runs at a time; running another, or the same again,
//! stops the one before. Unsaved files are saved first, as IntelliJ does,
//! so what runs is what's on screen.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::channel::mpsc::{TryRecvError, UnboundedReceiver};
use gpui_kit::component::input::{Copy, SelectAll};
use gpui_kit::component::native_menu::NativeMenu;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_editor::EditorHandle as _;

use super::{
    ChooseRunConfiguration, EditRunConfigurations, RunSelected, StopRun, TITLE_BAR_HEIGHT,
    ToggleRunPanel, Workspace,
};
use crate::run_configs::{self, RunConfig};
use crate::run_output::{Color, Link, OutputLine, Process, RunEvent, Stream, Style};
use crate::run_picker::{PLAY, RunPicker, RunPickerEvent};

/// The output panel's key context, where ⌘A selects all of it.
const OUTPUT_CONTEXT: &str = "JigRunOutput";

/// Select All in the output; Copy is bound for the whole window already.
pub(super) fn key_bindings() -> Vec<KeyBinding> {
    vec![KeyBinding::new(
        "secondary-a",
        SelectAll,
        Some(OUTPUT_CONTEXT),
    )]
}

/// Older lines are dropped past this many, so a chatty process can't eat
/// the machine's memory.
const MAX_LINES: usize = 50_000;
const LINE_HEIGHT: f32 = 18.;
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
    /// The configurations last loaded, and the project they're for.
    configs: Vec<RunConfig>,
    configs_root: Option<PathBuf>,
    _loading: Option<Task<()>>,
    /// What ⌃R runs.
    pub(super) selected: Option<RunConfig>,
    pub(super) session: Option<RunSession>,
    picker: Option<OpenRunPicker>,
    /// The picker was opened to debug what's chosen, not run it.
    picker_debug: bool,
    pub(super) panel_open: bool,
    panel_height: Pixels,
    /// Set while the panel's top edge is being dragged.
    resizing: bool,
    /// Set while output is being selected with the mouse.
    selecting: bool,
    /// The output's, so ⌘C and ⌘A reach it once it's clicked.
    output_focus: FocusHandle,
    next_id: u64,
}

impl RunState {
    pub(super) fn new(cx: &mut App) -> Self {
        Self {
            configs: Vec::new(),
            configs_root: None,
            _loading: None,
            selected: None,
            session: None,
            picker: None,
            picker_debug: false,
            panel_open: false,
            panel_height: px(220.),
            resizing: false,
            selecting: false,
            output_focus: cx.focus_handle(),
            next_id: 0,
        }
    }
}

/// One run of a configuration, and what it printed. Debugging is a run
/// with a debugger attached.
pub(super) struct RunSession {
    id: u64,
    pub(super) config: RunConfig,
    /// The program, or for a debug session, its build.
    pub(super) process: Option<Process>,
    events: UnboundedReceiver<RunEvent>,
    pub(super) lines: Vec<OutputLine>,
    /// How it ended, once it has.
    pub(super) ending: Option<String>,
    pub(super) started: Instant,
    scroll: UniformListScrollHandle,
    selection: Option<OutputSelection>,
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

struct OpenRunPicker {
    view: Entity<RunPicker>,
    _events: Subscription,
}

impl Workspace {
    /// The project configurations belong to: the sidebar's, or the open
    /// file's.
    pub(super) fn run_root(&self, cx: &App) -> Option<PathBuf> {
        self.project_root(cx).or_else(|| {
            self.document()
                .path
                .as_deref()
                .map(crate::project::root_for)
        })
    }

    pub(super) fn run_selected(
        &mut self,
        _: &RunSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.runs.selected.clone() {
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
    pub(super) fn run_file_saved(
        &mut self,
        path: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(root) = self.run_root(cx) else {
            return;
        };
        if super::tabs::canonical(path) == super::tabs::canonical(&root.join(run_configs::FILE)) {
            self.load_run_configs(root, window, cx);
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
        if self.runs.picker.is_some() {
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
        self.quick_open = None;
        self.find_in_files = None;
        let cached = if self.runs.configs_root.as_ref() == Some(&root) {
            self.runs.configs.clone()
        } else {
            Vec::new()
        };
        let current = self.runs.selected.as_ref().map(|c| c.name.clone());
        let view = cx.new(|cx| RunPicker::new(cached, true, current, window, cx));
        let events = cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
            RunPickerEvent::Run(index) => {
                let config = this.runs.configs.get(*index).cloned();
                this.close_run_picker(window, cx, true);
                if let Some(config) = config {
                    this.runs.selected = Some(config.clone());
                    if this.runs.picker_debug {
                        this.start_debug(config, window, cx);
                    } else {
                        this.start_run(config, window, cx);
                    }
                }
            }
            RunPickerEvent::Edit => {
                this.close_run_picker(window, cx, false);
                this.open_run_file(window, cx);
            }
            RunPickerEvent::Dismissed => this.close_run_picker(window, cx, true),
            RunPickerEvent::Blurred => this.close_run_picker(window, cx, false),
        });
        self.runs.picker = Some(OpenRunPicker {
            view,
            _events: events,
        });
        self.runs.picker_debug = debug;
        self.load_run_configs(root, window, cx);
        cx.notify();
    }

    fn close_run_picker(&mut self, window: &mut Window, cx: &mut Context<Self>, refocus: bool) {
        if self.runs.picker.take().is_some() {
            if refocus {
                self.focus_main(window, cx);
            }
            cx.notify();
        }
    }

    /// Ask the project what it can run, in the background, and pass the
    /// answer to the picker if it's open.
    fn load_run_configs(&mut self, root: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.runs._loading = Some(cx.spawn_in(window, async move |this, cx| {
            let load_root = root.clone();
            let (configs, error) = cx
                .background_executor()
                .spawn(async move { run_configs::load(&load_root) })
                .await;
            this.update_in(cx, |this, window, cx| {
                // The selection follows edits to its configuration.
                if let Some(selected) = &this.runs.selected
                    && let Some(fresh) = configs.iter().find(|c| c.name == selected.name)
                {
                    this.runs.selected = Some(fresh.clone());
                }
                this.runs.configs = configs.clone();
                this.runs.configs_root = Some(root);
                if let Some(picker) = &this.runs.picker {
                    picker
                        .view
                        .update(cx, |picker, cx| picker.set_configs(configs, cx));
                }
                if let Some(error) = error {
                    this.show_error(&format!("{error:#}"), window, cx);
                }
            })
            .ok();
        }));
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
        self.runs.session = Some(RunSession {
            id,
            config,
            process,
            events,
            lines,
            ending,
            started: Instant::now(),
            scroll: UniformListScrollHandle::new(),
            selection: None,
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
        let follow = scrolled_to_end(&session.scroll);
        let before = session.lines.len();
        for event in events {
            self.on_run_event(event, window, cx);
        }
        let messages = self.take_dap_messages();
        let any = !messages.is_empty();
        for (id, message) in messages {
            self.on_dap_message(id, message, window, cx);
        }
        let session = self.runs.session.as_mut().filter(|s| s.id == id)?;
        if session.lines.len() > MAX_LINES {
            let excess = session.lines.len() - MAX_LINES + MAX_LINES / 10;
            session.lines.drain(..excess);
            session.selection = session
                .selection
                .and_then(|selection| selection.shifted_up(excess));
        }
        if session.lines.len() != before || any {
            if follow {
                session.scroll.scroll_to_item(
                    session.lines.len().saturating_sub(1),
                    ScrollStrategy::Bottom,
                );
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
            RunEvent::Line(line) => session.lines.push(line),
            RunEvent::Data(json) => self.on_build_data(&json),
            RunEvent::Exited { message, success } => {
                if session.debug.is_some() {
                    let stopped = message == "Process stopped";
                    if success {
                        session.lines.push(OutputLine::meta("Build finished"));
                    } else if !stopped {
                        session.lines.push(OutputLine::meta(message));
                    }
                    self.on_build_exit(success, stopped, window, cx);
                    return;
                }
                session.process = None;
                let elapsed = session.started.elapsed().as_secs_f32();
                session
                    .lines
                    .push(OutputLine::meta(format!("{message} ({elapsed:.1}s)")));
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
        let picker = self.runs.picker.as_ref()?;
        Some(picker.view.read(cx).row_names())
    }

    /// The current run's id, output and ending.
    #[cfg(test)]
    pub(super) fn run_output(&self) -> Option<(u64, Vec<OutputLine>, Option<String>)> {
        let session = self.runs.session.as_ref()?;
        Some((session.id, session.lines.clone(), session.ending.clone()))
    }

    /// The picker, centred near the top of the window like Go to File.
    pub(super) fn render_run_picker(&self, window: &Window) -> Option<AnyElement> {
        let open = self.runs.picker.as_ref()?;
        let width = px(crate::run_picker::WIDTH);
        let left = ((window.viewport_size().width - width) / 2.).max(px(8.));
        Some(
            deferred(
                anchored()
                    .position(point(left, px(TITLE_BAR_HEIGHT + 24.)))
                    .snap_to_window_with_margin(px(8.))
                    .child(open.view.clone()),
            )
            .into_any_element(),
        )
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
            .runs
            .selected
            .as_ref()
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

        let workspace = cx.entity().downgrade();
        let list = uniform_list(
            "run-output",
            session.lines.len(),
            cx.processor(move |this, range: Range<usize>, _, cx| {
                let Some(session) = this.runs.session.as_ref() else {
                    return Vec::new();
                };
                range
                    .map(|ix| {
                        let line = &session.lines[ix];
                        let selected = session
                            .selection
                            .and_then(|selection| selection.range_in(ix, line.text.len()));
                        render_line(line, ix, selected, workspace.clone(), cx)
                    })
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&session.scroll)
        .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
        .flex_1()
        .min_h_0()
        .px_3()
        .pb_1()
        .font_family(theme.mono_font_family.clone())
        .text_size(px(12.));

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
                    None => this.child(self.output_area(list, cx)),
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

    /// The output, focusable so it takes ⌘C and ⌘A, with a menu to copy it.
    fn output_area(&self, list: impl IntoElement, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .id("run-output-area")
            .flex_1()
            .min_h_0()
            .key_context(OUTPUT_CONTEXT)
            .track_focus(&self.runs.output_focus)
            .on_action(cx.listener(Self::copy_output))
            .on_action(cx.listener(Self::select_all_output))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.runs.selecting = false),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    window.focus(&this.runs.output_focus, cx);
                    let has_selection = this
                        .runs
                        .session
                        .as_ref()
                        .and_then(|session| session.selection)
                        .is_some_and(|selection| !selection.is_empty());
                    NativeMenu::new()
                        .menu_with_disabled("Copy", !has_selection, Box::new(Copy))
                        .menu("Select All", Box::new(SelectAll))
                        .show(event.position, window, cx);
                }),
            )
            .child(list)
    }

    /// Start selecting output at the press, or extend the selection there
    /// with Shift. A double click takes the word, a triple the line.
    fn press_output(
        &mut self,
        point: (usize, usize),
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.runs.output_focus, cx);
        let Some(session) = self.runs.session.as_mut() else {
            return;
        };
        let (line, at) = point;
        let text = session.lines.get(line).map_or("", |l| l.text.as_ref());
        session.selection = Some(match (event.click_count, session.selection) {
            (1, Some(selection)) if event.modifiers.shift => OutputSelection {
                head: point,
                ..selection
            },
            (2, _) => {
                let word = word_at(text, at);
                OutputSelection {
                    anchor: (line, word.start),
                    head: (line, word.end),
                }
            }
            (3.., _) => OutputSelection {
                anchor: (line, 0),
                head: (line, text.len()),
            },
            _ => OutputSelection {
                anchor: point,
                head: point,
            },
        });
        self.runs.selecting = event.click_count == 1;
        cx.notify();
    }

    fn drag_output(&mut self, point: (usize, usize), dragging: bool, cx: &mut Context<Self>) {
        if !self.runs.selecting {
            return;
        }
        if !dragging {
            self.runs.selecting = false;
            return;
        }
        if let Some(selection) = self
            .runs
            .session
            .as_mut()
            .and_then(|session| session.selection.as_mut())
            && selection.head != point
        {
            selection.head = point;
            cx.notify();
        }
    }

    fn copy_output(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.runs.session.as_ref() else {
            return;
        };
        let Some(selection) = session.selection.filter(|s| !s.is_empty()) else {
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(selection.text(&session.lines)));
    }

    fn select_all_output(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.runs.session.as_mut() else {
            return;
        };
        let last = session.lines.len().saturating_sub(1);
        let end = session.lines.last().map_or(0, |line| line.text.len());
        session.selection = Some(OutputSelection {
            anchor: (0, 0),
            head: (last, end),
        });
        cx.notify();
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

/// Whether the list shows its last line, so new output should keep it in
/// view; scrolled up to read, it stays put.
fn scrolled_to_end(scroll: &UniformListScrollHandle) -> bool {
    let base = scroll.0.borrow().base_handle.clone();
    -base.offset().y >= base.max_offset().y - px(2.)
}

/// Output selected with the mouse, from where the drag began to where it
/// is now. Points are a line and a byte offset in it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct OutputSelection {
    anchor: (usize, usize),
    head: (usize, usize),
}

impl OutputSelection {
    fn ordered(self) -> ((usize, usize), (usize, usize)) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }

    fn is_empty(self) -> bool {
        self.anchor == self.head
    }

    /// What's selected of line `ix`, `len` bytes long.
    fn range_in(self, ix: usize, len: usize) -> Option<Range<usize>> {
        let (start, end) = self.ordered();
        if ix < start.0 || ix > end.0 {
            return None;
        }
        let from = if ix == start.0 { start.1.min(len) } else { 0 };
        let to = if ix == end.0 { end.1.min(len) } else { len };
        (from < to).then_some(from..to)
    }

    /// The selected text, lines joined by newlines.
    fn text(self, lines: &[OutputLine]) -> String {
        let (start, end) = self.ordered();
        let mut text = String::new();
        for (ix, line) in lines.iter().enumerate().take(end.0 + 1).skip(start.0) {
            if ix > start.0 {
                text.push('\n');
            }
            if let Some(range) = self.range_in(ix, line.text.len()) {
                text.push_str(&line.text[range]);
            }
        }
        text
    }

    /// The same text after the first `count` lines were dropped, unless
    /// it was among them.
    fn shifted_up(self, count: usize) -> Option<Self> {
        let shift = |(line, at): (usize, usize)| Some((line.checked_sub(count)?, at));
        Some(Self {
            anchor: shift(self.anchor)?,
            head: shift(self.head)?,
        })
    }
}

/// The word around byte `at` of `text`: letters, digits and underscores,
/// or the single character there when it's none of those.
fn word_at(text: &str, at: usize) -> Range<usize> {
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let at = at.min(text.len());
    // Past the end of the line, the last word counts as clicked.
    let at = match text[at..].chars().next() {
        Some(_) => at,
        None => text[..at].char_indices().last().map_or(at, |(i, _)| i),
    };
    let Some(here) = text[at..].chars().next() else {
        return at..at;
    };
    if !is_word(here) {
        return at..at + here.len_utf8();
    }
    let start = text[..at]
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_word(c))
        .last()
        .map_or(at, |(i, _)| i);
    let end = text[at..]
        .char_indices()
        .find(|&(_, c)| !is_word(c))
        .map_or(text.len(), |(i, _)| at + i);
    start..end
}

fn render_line(
    line: &OutputLine,
    ix: usize,
    selected: Option<Range<usize>>,
    workspace: WeakEntity<Workspace>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let color = match line.stream {
        Stream::Meta => theme.muted_foreground,
        Stream::Stdout | Stream::Stderr => theme.foreground,
    };
    let link = HighlightStyle {
        color: Some(theme.link),
        underline: Some(UnderlineStyle {
            thickness: px(1.),
            color: Some(theme.link.opacity(0.6)),
            wavy: false,
        }),
        ..Default::default()
    };
    let spans: Vec<(Range<usize>, HighlightStyle)> = line
        .spans
        .iter()
        .map(|(range, style)| (range.clone(), highlight(*style, cx)))
        .collect();
    let links: Vec<(Range<usize>, HighlightStyle)> =
        line.links.iter().map(|l| (l.range.clone(), link)).collect();
    let selection = HighlightStyle {
        background_color: Some(theme.selection),
        ..Default::default()
    };
    let selected: Vec<(Range<usize>, HighlightStyle)> = selected
        .map(|range| (range, selection))
        .into_iter()
        .collect();
    let text = StyledText::new(line.text.clone())
        .with_highlights(layer(&layer(&spans, &links), &selected));
    let layout = text.layout().clone();
    let point_at = move |position: Point<Pixels>| {
        let at = layout
            .index_for_position(position)
            .unwrap_or_else(|closest| closest);
        (ix, at)
    };
    let content: AnyElement = if line.links.is_empty() {
        text.into_any_element()
    } else {
        let targets = line.links.clone();
        let workspace = workspace.clone();
        InteractiveText::new(("run-line", ix), text)
            .on_click(
                targets.iter().map(|l| l.range.clone()).collect(),
                move |clicked, window, cx| {
                    let target = targets[clicked].clone();
                    workspace
                        .update(cx, |this, cx| this.open_link(&target, window, cx))
                        .ok();
                },
            )
            .into_any_element()
    };
    let on_press = workspace.clone();
    let point_at_press = point_at.clone();
    div()
        .h(px(LINE_HEIGHT))
        .whitespace_nowrap()
        .text_color(color)
        .cursor_text()
        .on_mouse_down(MouseButton::Left, move |event, window, cx| {
            let point = point_at_press(event.position);
            on_press
                .update(cx, |this, cx| this.press_output(point, event, window, cx))
                .ok();
        })
        .on_mouse_move(move |event, _, cx| {
            let point = point_at(event.position);
            workspace
                .update(cx, |this, cx| this.drag_output(point, event.dragging(), cx))
                .ok();
        })
        .child(content)
        .into_any_element()
}

fn highlight(style: Style, cx: &App) -> HighlightStyle {
    let theme = cx.theme();
    HighlightStyle {
        color: style.fg.map(|fg| match fg {
            Color::Indexed(n) => match n % 8 {
                0 => theme.muted_foreground,
                1 => theme.red,
                2 => theme.green,
                3 => theme.yellow,
                4 => theme.blue,
                5 => theme.magenta,
                6 => theme.cyan,
                _ => theme.foreground,
            },
            Color::Rgb(r, g, b) => rgb(u32::from_be_bytes([0, r, g, b])).into(),
        }),
        font_weight: style.bold.then_some(FontWeight::BOLD),
        font_style: style.italic.then_some(FontStyle::Italic),
        underline: style.underline.then_some(UnderlineStyle {
            thickness: px(1.),
            color: None,
            wavy: false,
        }),
        fade_out: style.dim.then_some(0.4),
        ..Default::default()
    }
}

/// `top` over `base`, as the sorted, non-overlapping ranges text styling
/// needs. Both are sorted and non-overlapping themselves.
fn layer(
    base: &[(Range<usize>, HighlightStyle)],
    top: &[(Range<usize>, HighlightStyle)],
) -> Vec<(Range<usize>, HighlightStyle)> {
    if top.is_empty() {
        return base.to_vec();
    }
    let mut cuts: Vec<usize> = base
        .iter()
        .chain(top)
        .flat_map(|(range, _)| [range.start, range.end])
        .collect();
    cuts.sort_unstable();
    cuts.dedup();
    let style_at = |ranges: &[(Range<usize>, HighlightStyle)], at: usize| {
        ranges
            .iter()
            .find(|(range, _)| range.contains(&at))
            .map(|(_, style)| *style)
    };
    cuts.windows(2)
        .filter_map(|pair| {
            let (start, end) = (pair[0], pair[1]);
            let style = match (style_at(base, start), style_at(top, start)) {
                (None, None) => return None,
                (Some(base), None) => base,
                (None, Some(top)) => top,
                (Some(base), Some(top)) => base.highlight(top),
            };
            Some((start..end, style))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use gpui_kit::{HighlightStyle, Hsla};

    use super::{OutputSelection, layer, word_at};
    use crate::run_output::OutputLine;

    #[test]
    fn links_layer_over_colours() {
        let red = HighlightStyle {
            color: Some(Hsla::red()),
            ..Default::default()
        };
        let link = HighlightStyle {
            color: Some(Hsla::blue()),
            ..Default::default()
        };
        let layered = layer(&[(0..10, red)], &[(4..14, link)]);
        assert_eq!(layered, [(0..4, red), (4..10, link), (10..14, link)]);
    }

    #[test]
    fn selection_copies_across_lines_either_way() {
        let lines = ["first line", "second", "third line"].map(OutputLine::meta);
        let down = OutputSelection {
            anchor: (0, 6),
            head: (2, 5),
        };
        assert_eq!(down.text(&lines), "line\nsecond\nthird");
        let up = OutputSelection {
            anchor: down.head,
            head: down.anchor,
        };
        assert_eq!(up.text(&lines), "line\nsecond\nthird");
        assert_eq!(down.range_in(1, 6), Some(0..6));
        assert_eq!(down.range_in(3, 4), None);
    }

    #[test]
    fn selection_follows_dropped_lines() {
        let selection = OutputSelection {
            anchor: (5, 1),
            head: (7, 2),
        };
        assert_eq!(
            selection.shifted_up(5),
            Some(OutputSelection {
                anchor: (0, 1),
                head: (2, 2),
            })
        );
        assert_eq!(selection.shifted_up(6), None);
    }

    #[test]
    fn double_click_takes_the_word() {
        let text = "error: cannot_find foo";
        assert_eq!(&text[word_at(text, 10)], "cannot_find");
        assert_eq!(&text[word_at(text, 5)], ":");
        assert_eq!(&text[word_at(text, text.len())], "foo");
    }
}
