//! Switching and creating branches, saving first and reloading tabs after.

use super::*;

impl Workspace {
    pub(in crate::workspace) fn switch_branch(
        &mut self,
        _: &SwitchBranch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_branch_picker(BranchPickerMode::Switch, window, cx);
    }

    pub(in crate::workspace) fn new_branch(
        &mut self,
        _: &NewBranch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_branch_picker(BranchPickerMode::Create, window, cx);
    }

    pub(super) fn open_branch_picker(
        &mut self,
        mode: BranchPickerMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.git.repo.clone() else {
            self.show_error("This project isn't in a Git repository.", window, cx);
            return;
        };
        self.palette = None;
        self.close_quick_open(cx);
        self.close_find_in_files(cx);
        let head = self.git.head.clone();
        self.branch_switcher.update(cx, |switcher, cx| {
            switcher.open(repo, head, mode, window, cx)
        });
        cx.notify();
    }

    /// Act on what the branch picker says: check out, or back to where the
    /// keyboard was.
    pub(in crate::workspace) fn on_branch_switcher_event(
        &mut self,
        _: &Entity<BranchSwitcher>,
        event: &BranchSwitcherEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            BranchSwitcherEvent::Switch(branch) => {
                self.focus_main(window, cx);
                cx.notify();
                if !branch.current {
                    let branch = branch.clone();
                    self.checkout(move |repo| repo.switch(&branch), window, cx);
                }
            }
            BranchSwitcherEvent::Create(name) => {
                self.focus_main(window, cx);
                cx.notify();
                let name = name.clone();
                self.checkout(move |repo| repo.create_branch(&name), window, cx);
            }
            BranchSwitcherEvent::Closed { refocus } => {
                if *refocus {
                    self.focus_main(window, cx);
                }
                cx.notify();
            }
        }
    }

    /// Save every changed file, then run `switch` in the background, then
    /// let open tabs follow what it did to the files.
    pub(super) fn checkout(
        &mut self,
        switch: impl FnOnce(&Repo) -> anyhow::Result<()> + Send + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(repo) = self.git.repo.clone() else {
            return;
        };
        if !self.save_all_for_git(window, cx) {
            return;
        }
        self.git._switch = Some(cx.spawn_in(window, async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { switch(&repo) })
                .await;
            this.update_in(cx, |this, window, cx| {
                if let Err(error) = result {
                    this.show_error(&format!("{error:#}"), window, cx);
                }
                this.git_changed(window, cx);
            })
            .ok();
        }));
    }

    /// Save each changed file that has somewhere to go, so git sees it.
    /// `false`, after saying so, if one couldn't be.
    fn save_all_for_git(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let mut failed = Vec::new();
        for ix in 0..self.tabs.len() {
            let tab = &self.tabs[ix];
            let Some(path) = tab.document.path.clone().filter(|_| tab.dirty) else {
                continue;
            };
            let text = tab.editor.text(cx);
            match self.tabs[ix].document.save(&path, &text) {
                Ok(()) => self.tabs[ix].dirty = false,
                Err(_) => failed.push(self.tabs[ix].document.title()),
            }
        }
        self.lsp_saved(cx);
        self.update_title(window);
        if !failed.is_empty() {
            self.show_error(
                &format!("Couldn't save {}, so the branch stays.", failed.join(", ")),
                window,
                cx,
            );
        }
        failed.is_empty()
    }

    /// Git changed files or the branch: open tabs with no unsaved changes
    /// show the files as they now are, and everything git-related refreshes.
    pub(super) fn git_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for ix in 0..self.tabs.len() {
            let tab = &self.tabs[ix];
            let Some(path) = tab.document.path.clone().filter(|_| !tab.dirty) else {
                continue;
            };
            let Ok(document) = Document::open(&path) else {
                // Not on this branch; the tab keeps what it had.
                continue;
            };
            if document.saved_text == tab.document.saved_text {
                continue;
            }
            self.remember_breakpoints(ix, cx);
            if ix == self.active {
                self.leave_tab(cx);
            }
            self.tabs[ix] = self.new_tab(document, window, cx);
            if ix == self.active {
                self.activate(ix, window, cx);
            }
        }
        self.refresh_git(window, cx);
        self.refresh_tree(cx);
        cx.notify();
    }
}
