//! Run configurations: choosing one (⌃⌥R), running it (⌃R), stopping it
//! (⌘F2), and the panel under the editor that shows its output (⌘J).
//!
//! One configuration runs at a time; running another, or the same again,
//! stops the one before. Unsaved files are saved first, as IntelliJ does,
//! so what runs is what's on screen.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::channel::mpsc::TryRecvError;
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
    selected: Option<RunConfig>,
    session: Option<RunSession>,
    picker: Option<OpenRunPicker>,
    panel_open: bool,
    panel_height: Pixels,
    /// Set while the panel's top edge is being dragged.
    resizing: bool,
    next_id: u64,
}

impl Default for RunState {
    fn default() -> Self {
        Self {
            configs: Vec::new(),
            configs_root: None,
            _loading: None,
            selected: None,
            session: None,
            picker: None,
            panel_open: false,
            panel_height: px(220.),
            resizing: false,
            next_id: 0,
        }
    }
}

/// One run of a configuration, and what it printed.
struct RunSession {
    id: u64,
    config: RunConfig,
    process: Option<Process>,
    lines: Vec<OutputLine>,
    /// How it ended, once it has.
    ending: Option<String>,
    started: Instant,
    scroll: UniformListScrollHandle,
    _events: Task<()>,
}

impl RunSession {
    fn is_running(&self) -> bool {
        self.ending.is_none() && self.process.as_ref().is_some_and(Process::is_running)
    }
}

struct OpenRunPicker {
    view: Entity<RunPicker>,
    _events: Subscription,
}

impl Workspace {
    /// The project configurations belong to: the sidebar's, or the open
    /// file's.
    fn run_root(&self, cx: &App) -> Option<PathBuf> {
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

    pub(super) fn stop_process(&self) {
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
                    this.start_run(config, window, cx);
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
        self.save_all_for_run(window, cx);
        // Stopped by being dropped.
        self.runs.session = None;
        self.runs.next_id += 1;
        let id = self.runs.next_id;
        let root = self.run_root(cx).unwrap_or_else(|| config.cwd.clone());

        let (sender, mut events) = futures::channel::mpsc::unbounded();
        let mut lines = vec![OutputLine::meta(format!("$ {}", config.command))];
        let mut ending = None;
        let process = match Process::spawn(&config, &root, sender) {
            Ok(process) => Some(process),
            Err(error) => {
                let message = format!("{error:#}");
                lines.push(OutputLine::meta(message.clone()));
                ending = Some(message);
                None
            }
        };
        // Polled rather than awaited: output comes from the process's own
        // threads, and a timer batches a flood of it into fewer repaints.
        let task = cx.spawn(async move |this, cx| {
            loop {
                let mut batch = Vec::new();
                let mut closed = false;
                while batch.len() < MAX_BATCH {
                    match events.try_recv() {
                        Ok(event) => batch.push(event),
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Closed) => {
                            closed = true;
                            break;
                        }
                    }
                }
                let full = batch.len() == MAX_BATCH;
                if !batch.is_empty()
                    && this
                        .update(cx, |this, cx| this.on_run_events(id, batch, cx))
                        .is_err()
                {
                    break;
                }
                if closed {
                    break;
                }
                if !full {
                    cx.background_executor().timer(POLL_INTERVAL).await;
                }
            }
        });
        self.runs.session = Some(RunSession {
            id,
            config,
            process,
            lines,
            ending,
            started: Instant::now(),
            scroll: UniformListScrollHandle::new(),
            _events: task,
        });
        self.runs.panel_open = true;
        cx.notify();
    }

    fn on_run_events(&mut self, id: u64, events: Vec<RunEvent>, cx: &mut Context<Self>) {
        let Some(session) = self.runs.session.as_mut().filter(|s| s.id == id) else {
            return;
        };
        let follow = scrolled_to_end(&session.scroll);
        for event in events {
            match event {
                RunEvent::Line(line) => session.lines.push(line),
                RunEvent::Exited(ending) => {
                    let elapsed = session.started.elapsed().as_secs_f32();
                    session
                        .lines
                        .push(OutputLine::meta(format!("{ending} ({elapsed:.1}s)")));
                    session.ending = Some(ending);
                }
            }
        }
        if session.lines.len() > MAX_LINES {
            let excess = session.lines.len() - MAX_LINES + MAX_LINES / 10;
            session.lines.drain(..excess);
        }
        if follow {
            session.scroll.scroll_to_item(
                session.lines.len().saturating_sub(1),
                ScrollStrategy::Bottom,
            );
        }
        cx.notify();
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
                .hover(|s| s.bg(theme.foreground.opacity(0.08)))
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
                        .h(px(26.))
                        .max_w(px(200.))
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
                        .child(div().truncate().child(name))
                        .child(
                            Icon::default()
                                .data(CHEVRON)
                                .size(px(10.))
                                .flex_none()
                                .text_color(theme.muted_foreground),
                        )
                        .on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(ChooseRunConfiguration), cx)
                        }),
                )
                .child(
                    button(
                        "run-selected",
                        if running { RERUN } else { PLAY },
                        theme.success,
                    )
                    .on_click(|_, window, cx| window.dispatch_action(Box::new(RunSelected), cx)),
                )
                .when(running, |this| {
                    this.child(
                        button("stop-run", STOP, theme.danger).on_click(|_, window, cx| {
                            window.dispatch_action(Box::new(StopRun), cx)
                        }),
                    )
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
        let status = match &session.ending {
            Some(ending) => ending.clone(),
            None => "Running…".into(),
        };
        let status_color = match &session.ending {
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
            .child(
                button("panel-rerun", RERUN, theme.success).on_click(cx.listener(
                    |this, _, window, cx| {
                        if let Some(config) = this.runs.session.as_ref().map(|s| s.config.clone()) {
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
                    .map(|ix| render_line(&session.lines[ix], ix, workspace.clone(), cx))
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
                .bg(theme.background)
                .border_t_1()
                .border_color(theme.title_bar_border)
                .child(header)
                .child(list)
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

/// Whether the list shows its last line, so new output should keep it in
/// view; scrolled up to read, it stays put.
fn scrolled_to_end(scroll: &UniformListScrollHandle) -> bool {
    let base = scroll.0.borrow().base_handle.clone();
    -base.offset().y >= base.max_offset().y - px(2.)
}

fn render_line(
    line: &OutputLine,
    ix: usize,
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
    let text = StyledText::new(line.text.clone()).with_highlights(layer(&spans, &links));
    let content: AnyElement = if line.links.is_empty() {
        text.into_any_element()
    } else {
        let targets = line.links.clone();
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
    div()
        .h(px(LINE_HEIGHT))
        .whitespace_nowrap()
        .text_color(color)
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

    use super::layer;

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
}
