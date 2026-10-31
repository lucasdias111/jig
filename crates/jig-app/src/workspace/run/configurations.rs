//! The project's run configurations, the one ⌃R runs, and the picker that
//! chooses it (⌃⌥R). An entity of its own: the workspace opens the picker,
//! places it, and runs or debugs what it says was chosen.

use std::path::PathBuf;

use gpui_kit::*;

use super::super::TITLE_BAR_HEIGHT;
use jig_run::run_configs::{self, RunConfig};
use jig_run::run_picker::{self, RunPicker, RunPickerEvent};

#[derive(Default)]
pub(in crate::workspace) struct RunConfigurations {
    /// The configurations last loaded, and the project they're for.
    configs: Vec<RunConfig>,
    root: Option<PathBuf>,
    _loading: Option<Task<()>>,
    /// What ⌃R runs.
    selected: Option<RunConfig>,
    picker: Option<OpenPicker>,
}

struct OpenPicker {
    view: Entity<RunPicker>,
    /// It was opened to debug what's chosen, not run it.
    debug: bool,
    _events: Subscription,
}

/// What the configurations ask of the workspace, which runs things and
/// has the focus.
pub(in crate::workspace) enum RunConfigurationsEvent {
    /// `config` was chosen in the picker, which has closed, to run it or
    /// with `debug`, to debug it. It's now what ⌃R runs.
    Chosen { config: RunConfig, debug: bool },
    /// Edit `run.toml` was chosen in the picker, which has closed.
    Edit,
    /// The picker closed without a choice. `refocus` when it was
    /// dismissed, rather than left by clicking elsewhere.
    Closed { refocus: bool },
    /// The project's configurations couldn't all be read.
    LoadFailed(String),
}

impl EventEmitter<RunConfigurationsEvent> for RunConfigurations {}

impl RunConfigurations {
    /// What ⌃R runs, once one has been chosen.
    pub(in crate::workspace) fn selected(&self) -> Option<&RunConfig> {
        self.selected.as_ref()
    }

    pub(in crate::workspace) fn picker_open(&self) -> bool {
        self.picker.is_some()
    }

    /// Open the picker on `root`'s configurations, to run what's chosen or
    /// with `debug`, to debug it. It shows those last loaded for `root` at
    /// once, and the fresh ones once they're read.
    pub(in crate::workspace) fn open_picker(
        &mut self,
        root: PathBuf,
        debug: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let cached = if self.root.as_ref() == Some(&root) {
            self.configs.clone()
        } else {
            Vec::new()
        };
        let current = self.selected.as_ref().map(|c| c.name.clone());
        let view = cx.new(|cx| RunPicker::new(cached, true, current, window, cx));
        let events = cx.subscribe(&view, |this, _, event, cx| match event {
            RunPickerEvent::Run(index) => {
                let Some(picker) = this.picker.take() else {
                    return;
                };
                match this.configs.get(*index).cloned() {
                    Some(config) => {
                        this.selected = Some(config.clone());
                        cx.emit(RunConfigurationsEvent::Chosen {
                            config,
                            debug: picker.debug,
                        });
                    }
                    None => cx.emit(RunConfigurationsEvent::Closed { refocus: true }),
                }
                cx.notify();
            }
            RunPickerEvent::Edit => {
                if this.picker.take().is_some() {
                    cx.emit(RunConfigurationsEvent::Edit);
                    cx.notify();
                }
            }
            RunPickerEvent::Dismissed => this.close_picker(true, cx),
            RunPickerEvent::Blurred => this.close_picker(false, cx),
        });
        self.picker = Some(OpenPicker {
            view,
            debug,
            _events: events,
        });
        self.load(root, cx);
        cx.notify();
    }

    fn close_picker(&mut self, refocus: bool, cx: &mut Context<Self>) {
        if self.picker.take().is_some() {
            cx.emit(RunConfigurationsEvent::Closed { refocus });
            cx.notify();
        }
    }

    /// Ask the project what it can run, in the background, and pass the
    /// answer to the picker if it's open.
    pub(in crate::workspace) fn load(&mut self, root: PathBuf, cx: &mut Context<Self>) {
        self._loading = Some(cx.spawn(async move |this, cx| {
            let load_root = root.clone();
            let (configs, error) = cx
                .background_executor()
                .spawn(async move { run_configs::load(&load_root) })
                .await;
            this.update(cx, |this, cx| {
                // The selection follows edits to its configuration.
                if let Some(selected) = &this.selected
                    && let Some(fresh) = configs.iter().find(|c| c.name == selected.name)
                {
                    this.selected = Some(fresh.clone());
                }
                this.configs = configs.clone();
                this.root = Some(root);
                if let Some(picker) = &this.picker {
                    picker
                        .view
                        .update(cx, |picker, cx| picker.set_configs(configs, cx));
                }
                if let Some(error) = error {
                    cx.emit(RunConfigurationsEvent::LoadFailed(format!("{error:#}")));
                }
                cx.notify();
            })
            .ok();
        }));
    }

    /// Forget everything, for another project.
    pub(in crate::workspace) fn reset(&mut self, cx: &mut Context<Self>) {
        *self = Self::default();
        cx.notify();
    }

    #[cfg(test)]
    pub(in crate::workspace) fn picker_rows(&self, cx: &App) -> Option<Vec<String>> {
        let picker = self.picker.as_ref()?;
        Some(picker.view.read(cx).row_names())
    }
}

impl Render for RunConfigurations {
    /// The picker, centred near the top of the window like Go to File.
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let Some(picker) = &self.picker else {
            return div().into_any_element();
        };
        let width = px(run_picker::WIDTH);
        let left = ((window.viewport_size().width - width) / 2.).max(px(8.));
        deferred(
            anchored()
                .position(point(left, px(TITLE_BAR_HEIGHT + 24.)))
                .snap_to_window_with_margin(px(8.))
                .child(picker.view.clone()),
        )
        .into_any_element()
    }
}
