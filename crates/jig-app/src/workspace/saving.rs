//! Saving, Save As, and asking about unsaved changes before closing, quitting or updating.

use super::*;

impl Workspace {
    /// Run `then` now if there are no unsaved changes, otherwise only after
    /// the user agrees to discard them. Checks the tabs `only`, or every tab.
    pub(super) fn when_discard_ok(
        &mut self,
        only: Option<&[EntityId]>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let dirty: Vec<String> = self
            .tabs
            .iter()
            .filter(|tab| tab.dirty && only.is_none_or(|ids| ids.contains(&tab.id())))
            .map(|tab| tab.document.title())
            .collect();
        let detail = match dirty.as_slice() {
            [] => {
                then(self, window, cx);
                return;
            }
            [title] => format!("{title} has unsaved changes."),
            titles => format!("{} files have unsaved changes.", titles.len()),
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            "Discard unsaved changes?",
            Some(&detail),
            &["Discard", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |this, window, cx| then(this, window, cx))
                    .ok();
            }
        })
        .detach();
    }

    pub(super) fn confirm_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.any_dirty() {
            return true;
        }
        self.close_after_discard(window, cx);
        false
    }

    fn close_after_discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.when_discard_ok(None, window, cx, |this, window, _| {
            // So closing doesn't ask again.
            for tab in &mut this.tabs {
                tab.dirty = false;
            }
            window.remove_window();
        });
    }

    pub(super) fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.home {
            return;
        }
        match self.document().path.clone() {
            Some(path) => self.save_tidied(path, window, cx),
            None => self.prompt_save_as(window, cx),
        }
    }

    pub(super) fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        if self.home {
            return;
        }
        self.prompt_save_as(window, cx);
    }

    fn prompt_save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self
            .document()
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let name = self.document().title();
        let path = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| this.save_tidied(path, window, cx))
                .ok();
        })
        .detach();
    }

    pub(super) fn save_to(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor().text(cx);
        let languages = &self.settings.languages;
        let language_changed =
            jig_languages::language_for(path, languages) != self.document().language(languages);
        if let Err(error) = self.tab_mut().document.save(path, &text) {
            self.show_error(&format!("Could not save: {error:#}"), window, cx);
            return;
        }
        if language_changed {
            // Rebuild so highlighting matches the new extension.
            self.reload_tab(path, window, cx);
        }
        self.tab_mut().dirty = false;
        self.lsp_saved(cx);
        self.update_title(window);
        if self.is_config_file() {
            jig_agent::providers::reload(cx);
        }
        self.run_file_saved(path, cx);
        self.refresh_git_panel(window, cx);
        self.remember_breakpoints(self.active, cx);
        // Save As may have added a file.
        self.refresh_tree(cx);
        cx.notify();
    }

    pub(super) fn close_window(
        &mut self,
        _: &CloseWindow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_after_discard(window, cx);
    }

    pub(super) fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        self.when_discard_ok(None, window, cx, |_, _, cx| cx.quit());
    }

    /// Install the downloaded update and start it on this window's project
    /// or file, as quitting would: unsaved changes are asked about first.
    pub(super) fn restart_to_update(
        &mut self,
        _: &jig_updates::RestartToUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.when_discard_ok(None, window, cx, |this, window, cx| {
            let reopen = this
                .project_root(cx)
                .or_else(|| this.document().path.clone());
            if let Err(error) = jig_updates::restart(reopen, cx) {
                this.show_error(&error, window, cx);
            }
        });
    }
}
