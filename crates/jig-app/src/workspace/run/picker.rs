//! Run configurations: the picker, the configuration ⌃R runs, and .jig/run.toml.

use super::*;

impl Workspace {
    pub(in crate::workspace) fn run_selected(
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

    pub(in crate::workspace) fn choose_run_configuration(
        &mut self,
        _: &ChooseRunConfiguration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_run_picker(window, cx);
    }

    pub(in crate::workspace) fn edit_run_configurations(
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
    pub(in crate::workspace) fn run_file_saved(&mut self, path: &Path, cx: &mut Context<Self>) {
        let Some(root) = self.run_root(cx) else {
            return;
        };
        if super::super::tabs::canonical(path)
            == super::super::tabs::canonical(&root.join(run_configs::FILE))
        {
            self.run_configs
                .update(cx, |configurations, cx| configurations.load(root, cx));
        }
    }

    pub(super) fn open_run_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_run_picker_for(false, window, cx);
    }

    /// The picker, to run what's chosen or with `debug`, to debug it.
    pub(in crate::workspace) fn open_run_picker_for(
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
    pub(in crate::workspace) fn on_run_configurations_event(
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
}
