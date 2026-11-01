//! The rows for a language's tools: its server, formatters and debugger,
//! each with Install and Uninstall.

use super::*;

impl SettingsWindow {
    /// The language's server: the program Jig runs, or how to get it, with
    /// Install when Jig can fetch it and Uninstall once it has.
    pub(super) fn server_section(&self, language: &'static str, cx: &Context<Self>) -> Section {
        let section = Section::titled("Language server");
        let Some(server) = lsp::server_for(language) else {
            return section.footer(
                "Jig knows no language server for it, so go to definition and completion use \
                 words from the file.",
            );
        };
        let shared = names(
            &server
                .languages
                .iter()
                .filter(|other| *other != language)
                .cloned()
                .collect::<Vec<_>>(),
        );
        let footer = if shared.is_empty() {
            "Go to definition, completion, problems, hover and rename.".to_string()
        } else {
            format!(
                "Go to definition, completion, problems, hover and rename. It serves {shared} too."
            )
        };
        section.footer(footer).row(self.server_row(server, cx))
    }

    pub(super) fn server_row(&self, server: &'static Server, cx: &Context<Self>) -> Row {
        let (status, ready) = lsp::install::status(server);
        let key = format!("server-{}", server.name);
        let (description, ok, busy) = progress(self.installs.get(&key), status, ready);
        let manage = Manage {
            id: key.clone(),
            busy,
            installable: server.installable(),
            ready,
            removable: lsp::install::installed_by_jig(server),
            install: self.job_starter(
                &key,
                "Installing…",
                move || lsp::install::install(server).map(|()| "Installed."),
                lsp::servers_changed,
                cx,
            ),
            uninstall: self.job_starter(
                &key,
                "Moving to the Trash…",
                move || lsp::install::uninstall(server).map(|()| "Moved to the Trash."),
                lsp::servers_changed,
                cx,
            ),
        };
        let id = server.name.as_str();
        Row::new(key.clone(), id)
            .description(description)
            .warning(!ok)
            .keywords(["language server", "lsp", "install", "uninstall", id])
            .control(move |_, _| manage.render())
    }

    /// Which formatter the language uses, each one it could with Install
    /// and Uninstall, and formatting on save.
    pub(super) fn formatting_section(
        &self,
        language: &'static Language,
        cx: &Context<Self>,
    ) -> Section {
        let name = language.name;
        let registry = formatters::registry();
        let serving: Vec<Arc<Formatter>> = registry.serving(name).cloned().collect();
        let mut section = Section::titled("Formatting");
        if !serving.is_empty() {
            let mut options: Vec<(SharedString, SharedString)> =
                vec![("".into(), "Language server".into())];
            options.extend(
                serving
                    .iter()
                    .map(|f| (f.key.clone().into(), f.name.clone().into())),
            );
            section = section.row(
                Row::new(format!("{name}-formatter"), "Formatter")
                    .description(formatter_note(language, cx))
                    .keywords(["format", "formatter", name])
                    .control(ui::dropdown(
                        format!("{name}-formatter"),
                        options,
                        move |cx| {
                            settings::get(cx)
                                .formatting
                                .chosen(name)
                                .unwrap_or_default()
                                .to_string()
                                .into()
                        },
                        move |key, cx| {
                            let key = (!key.is_empty()).then_some(key.as_ref());
                            settings::update(cx, |s| s.formatting.choose(name, key))
                        },
                    )),
            );
            section = section.rows(serving.into_iter().map(|f| self.formatter_row(f, cx)));
        }
        section.row(
            Row::new(format!("{name}-format"), "Format on save")
                .description_opt(serving_note(&registry, language, cx))
                .keywords(["format", "formatter", "save", name])
                .control(ui::switch(
                    format!("{name}-format"),
                    move |cx| settings::get(cx).formatting.on_save(name),
                    move |on, cx| settings::update(cx, |s| s.formatting.set_on_save(name, on)),
                )),
        )
    }

    pub(super) fn formatter_row(&self, formatter: Arc<Formatter>, cx: &Context<Self>) -> Row {
        let (status, ready) = formatter.status();
        let key = format!("formatter-{}", formatter.key);
        let (description, ok, busy) = progress(self.installs.get(&key), status, ready);
        let (install, uninstall) = (formatter.clone(), formatter.clone());
        let manage = Manage {
            id: key.clone(),
            busy,
            installable: formatter.installable(),
            ready,
            removable: formatter.installed_by_jig(),
            install: self.job_starter(
                &key,
                "Installing…",
                move || install.install().map(|()| "Installed."),
                |_| {},
                cx,
            ),
            uninstall: self.job_starter(
                &key,
                "Moving to the Trash…",
                move || uninstall.uninstall().map(|()| "Moved to the Trash."),
                |_| {},
                cx,
            ),
        };
        Row::new(key.clone(), formatter.name.clone())
            .description(description)
            .warning(!ok)
            .keywords(["format", "formatter", "install", "uninstall"])
            .control(move |_, _| manage.render())
    }

    /// A debugger's switch, where Jig found it or what's missing, and
    /// Install when Jig can fetch it; once it's there, a menu to install
    /// it again or move what Jig installed to the Trash.
    pub(super) fn debugger_row(&self, debugger: Arc<Debugger>, cx: &Context<Self>) -> Row {
        let (status, ready) = debuggers::status(&debugger);
        let key = format!("debugger-{}", debugger.key);
        let (description, ok, busy) = progress(self.installs.get(&key), status, ready);
        // Turned on once it's there; off unless another copy is found.
        let settle = {
            let debugger = debugger.clone();
            move |cx: &mut App| {
                let (_, ready) = debuggers::status(&debugger);
                settings::update(cx, |s| s.debugging.set_enabled(&debugger.key, ready));
            }
        };
        let (install, uninstall) = (debugger.clone(), debugger.clone());
        let manage = Manage {
            id: key.clone(),
            busy,
            installable: debugger.installable(),
            ready,
            removable: !debuggers::installed_files(&debugger).is_empty(),
            install: self.job_starter(
                &key,
                "Installing…",
                move || debuggers::install(&install).map(|()| "Installed."),
                settle.clone(),
                cx,
            ),
            uninstall: self.job_starter(
                &key,
                "Moving to the Trash…",
                move || debuggers::uninstall(&uninstall).map(|()| "Moved to the Trash."),
                settle,
                cx,
            ),
        };
        let switch_key = debugger.key.clone();
        let (get, set) = (switch_key.clone(), switch_key);
        let switch = ui::switch(
            key.clone(),
            move |cx| settings::get(cx).debugging.is_enabled(&get),
            move |on, cx| settings::update(cx, |s| s.debugging.set_enabled(&set, on)),
        );
        Row::new(key, debugger.name.clone())
            .description(description)
            .warning(!ok)
            .keywords(["debug", "debugger", "breakpoint", "install", "uninstall"])
            .control(move |window, cx| manage.render().child(switch(window, cx)))
    }

    /// A click handler running `run` in the background under `key`, its
    /// progress shown on the tool's row, then `settle` with what changed.
    pub(super) fn job_starter<R, S>(
        &self,
        key: &str,
        doing: &'static str,
        run: R,
        settle: S,
        cx: &Context<Self>,
    ) -> Rc<dyn Fn(&mut App)>
    where
        R: Fn() -> anyhow::Result<&'static str> + Clone + Send + 'static,
        S: Fn(&mut App) + Clone + 'static,
    {
        let this = cx.entity().downgrade();
        let key = key.to_string();
        Rc::new(move |cx: &mut App| {
            let (key, run, settle) = (key.clone(), run.clone(), settle.clone());
            this.update(cx, |this, cx| this.tool_job(key, doing, run, settle, cx))
                .ok();
        })
    }

    pub(super) fn tool_job(
        &mut self,
        key: String,
        doing: &'static str,
        run: impl FnOnce() -> anyhow::Result<&'static str> + Send + 'static,
        settle: impl FnOnce(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if matches!(self.installs.get(&key), Some(Fetch::Running(_))) {
            return;
        }
        self.installs
            .insert(key.clone(), Fetch::Running(doing.into()));
        cx.spawn(async move |this, cx| {
            let result = cx.background_executor().spawn(async move { run() }).await;
            this.update(cx, |this, cx| {
                let fetch = match result {
                    Ok(done) => Fetch::Done(done.into()),
                    Err(error) => Fetch::Failed(format!("{error:#}").into()),
                };
                settle(cx);
                this.installs.insert(key, fetch);
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}
