//! Opening files and project folders, and switching the window to another project.

use super::*;

impl Workspace {
    /// Ask for a folder, or with `files`, a file or folder, and open it.
    pub(super) fn prompt_open(&mut self, files: bool, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                if path.is_dir() {
                    this.open_project(&path, window, cx);
                } else {
                    this.open_file(&path, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Open the folder `dir` as the project. With another project already
    /// open, ask whether it replaces that one or gets a window of its own.
    pub(crate) fn open_project(&mut self, dir: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let dir = tabs::canonical(dir);
        let Some(current) = self.project_root(cx).filter(|root| *root != dir) else {
            self.open_folder(&dir, window, cx);
            return;
        };
        let name = |path: &Path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        };
        let detail = format!("This window has {} open.", name(&current));
        let answer = window.prompt(
            PromptLevel::Info,
            &format!("Open {} in this window or a new one?", name(&dir)),
            Some(&detail),
            &["This Window", "New Window", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| match answer.await {
            Ok(0) => {
                this.update_in(cx, |this, window, cx| this.replace_project(dir, window, cx))
                    .ok();
            }
            Ok(1) => {
                cx.update(|_, cx| {
                    if let Err(error) = crate::open_window(Some(dir), &[], cx) {
                        eprintln!("jig: the new window didn't open: {error:#}");
                    }
                })
                .ok();
            }
            _ => {}
        })
        .detach();
    }

    /// Close this window's project, tabs and run included, and open `dir`
    /// in its place.
    fn replace_project(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.when_discard_ok(None, window, cx, move |this, window, cx| {
            this.leave_tab(cx);
            this.stop_process();
            this.runs = run::RunState::new();
            this.run_output
                .update(cx, |output, cx| output.start(Vec::new(), cx));
            this.run_configs
                .update(cx, |configurations, cx| configurations.reset(cx));
            this.palette = None;
            this.quick_open.update(cx, |panel, cx| panel.reset(cx));
            this.close_find_in_files(cx);
            this.file_index = None;
            let tab = this.new_tab(Document::default(), window, cx);
            this.tabs = vec![tab];
            this.active = 0;
            this.home = true;
            this.tree = None;
            this.open_folder(&dir, window, cx);
            this.update_title(window);
        });
    }

    pub(super) fn open(&mut self, _: &Open, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_open(true, window, cx);
    }
}
