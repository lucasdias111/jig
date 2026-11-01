//! The Git view in the sidebar, and placing the branch picker and the change popup.

use super::*;

impl Workspace {
    /// The change popup, while it's open on a tab that's still there.
    pub(in crate::workspace) fn render_hunk_popup(&self, cx: &App) -> Option<AnyElement> {
        let tab = self.hunk_popup.read(cx).tab()?;
        self.tabs
            .iter()
            .any(|t| t.id() == tab)
            .then(|| self.hunk_popup.clone().into_any_element())
    }

    /// ⌃⇧G: the sidebar's Git view, or collapsing the sidebar.
    pub(in crate::workspace) fn toggle_git_panel(
        &mut self,
        _: &ToggleGitPanel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tree.is_none() {
            self.show_error("Open a project first, to see its changes.", window, cx);
            return;
        }
        self.toggle_sidebar_view(SidebarView::Git, window, cx);
    }

    /// The sidebar has just switched to Git: make the panel if there's a
    /// repository, bring its lists up to date, and focus the message.
    pub(in crate::workspace) fn show_git_view(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.hunk_popup.update(cx, |popup, cx| popup.close(cx));
        if !self.ensure_git_panel(window, cx) {
            self.refresh_git_panel(window, cx);
        }
        if let Some(panel) = &self.git.panel {
            panel.view.update(cx, |panel, cx| panel.focus(window, cx));
        }
    }

    /// Make the Git view's panel if there's a repository and no panel yet.
    /// `true` if it made one, which reads the status as it starts.
    pub(super) fn ensure_git_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.git.panel.is_some() {
            return false;
        }
        let Some(repo) = self.git.repo.clone() else {
            return false;
        };
        let view = cx.new(|cx| GitPanel::new(repo, window, cx));
        let events = cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
            GitPanelEvent::Open(path) => {
                let path = path.clone();
                this.open_file(&path, window, cx);
            }
            GitPanelEvent::Changed => this.git_changed(window, cx),
            GitPanelEvent::Dismissed => this.leave_sidebar(window, cx),
            GitPanelEvent::Branches => {
                this.open_branch_picker(BranchPickerMode::Switch, window, cx)
            }
        });
        self.git.panel = Some(Opened {
            view,
            _events: events,
        });
        true
    }

    /// The sidebar's Git view, or why there's nothing in it.
    pub(in crate::workspace) fn render_git_view(&self, cx: &Context<Self>) -> AnyElement {
        if let Some(panel) = &self.git.panel {
            return panel.view.clone().into_any_element();
        }
        let theme = cx.theme();
        let message = if self.git.repo.is_some() {
            "Loading…"
        } else {
            "This project isn't in a Git repository. Run git init in its folder to start one."
        };
        div()
            .p_4()
            .text_size(px(12.5))
            .text_color(theme.muted_foreground)
            .child(message)
            .into_any_element()
    }

    /// Ask the Git view's panel for the status again, e.g. after a save.
    pub(in crate::workspace) fn refresh_git_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(panel) = &self.git.panel {
            panel.view.update(cx, |panel, cx| panel.refresh(window, cx));
        }
    }

    /// The branch picker, while it's open.
    pub(in crate::workspace) fn render_branch_picker(&self, cx: &App) -> Option<AnyElement> {
        self.branch_switcher
            .read(cx)
            .is_open()
            .then(|| self.branch_switcher.clone().into_any_element())
    }

    /// The branch, in the status bar; clicking it opens the branch picker.
    pub(in crate::workspace) fn render_branch_switcher(
        &self,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        self.git.repo.as_ref()?;
        let head = self.git.head.clone().unwrap_or_else(|| "Git".into());
        let theme = cx.theme();
        Some(
            h_flex()
                .id("branch-switcher")
                .debug_selector(|| "branch-switcher".into())
                .flex_none()
                .h_full()
                .max_w(px(220.))
                .px_1p5()
                .gap_1()
                .hover(|s| {
                    s.bg(theme.foreground.opacity(0.08))
                        .text_color(theme.foreground)
                })
                .child(Icon::default().data(BRANCH_ICON).size(px(12.)).flex_none())
                .child(div().min_w_0().truncate().whitespace_nowrap().child(head))
                .on_click(cx.listener(|this, _, window, cx| {
                    this.open_branch_picker(BranchPickerMode::Switch, window, cx)
                }))
                .into_any_element(),
        )
    }
}
