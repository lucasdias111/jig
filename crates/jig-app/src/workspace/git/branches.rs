//! The branch picker (Git > Switch Branch…, New Branch…, the status bar's
//! branch): an entity of its own, so the workspace only opens it, places
//! it and checks out what it says.

use gpui_kit::*;

use super::super::TITLE_BAR_HEIGHT;
use jig_git::{Branch, BranchPicker, BranchPickerEvent, BranchPickerMode, Repo};

#[derive(Default)]
pub(in crate::workspace) struct BranchSwitcher {
    open: Option<OpenPicker>,
}

struct OpenPicker {
    view: Entity<BranchPicker>,
    _events: Subscription,
}

/// What the picker asks of the workspace, which saves files and checks
/// out.
pub(in crate::workspace) enum BranchSwitcherEvent {
    /// Check out `branch`. The picker has closed.
    Switch(Branch),
    /// Start a branch named so from the current one. The picker has closed.
    Create(String),
    /// The picker closed without a choice. `refocus` when it was
    /// dismissed, rather than left by clicking elsewhere.
    Closed { refocus: bool },
}

impl EventEmitter<BranchSwitcherEvent> for BranchSwitcher {}

impl BranchSwitcher {
    /// The picker, while it's open.
    #[cfg(test)]
    pub(in crate::workspace) fn picker(&self) -> Option<&Entity<BranchPicker>> {
        self.open.as_ref().map(|open| &open.view)
    }

    pub(in crate::workspace) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Open the picker on `repo`'s branches, `head` checked out, its
    /// branches read in the background.
    pub(in crate::workspace) fn open(
        &mut self,
        repo: Repo,
        head: Option<String>,
        mode: BranchPickerMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.new(|cx| BranchPicker::new(mode, head, window, cx));
        let events = cx.subscribe(&view, |this, _, event, cx| match event {
            BranchPickerEvent::Switch(branch) => {
                this.close_with(BranchSwitcherEvent::Switch(branch.clone()), cx)
            }
            BranchPickerEvent::Create(name) => {
                this.close_with(BranchSwitcherEvent::Create(name.clone()), cx)
            }
            BranchPickerEvent::Dismissed => {
                this.close_with(BranchSwitcherEvent::Closed { refocus: true }, cx)
            }
            BranchPickerEvent::Blurred => {
                this.close_with(BranchSwitcherEvent::Closed { refocus: false }, cx)
            }
        });
        self.open = Some(OpenPicker {
            view: view.clone(),
            _events: events,
        });
        cx.spawn(async move |_, cx| {
            let branches = cx
                .background_executor()
                .spawn(async move { repo.branches().unwrap_or_default() })
                .await;
            view.update(cx, |picker, cx| picker.set_branches(branches, cx));
        })
        .detach();
        cx.notify();
    }

    fn close_with(&mut self, event: BranchSwitcherEvent, cx: &mut Context<Self>) {
        if self.open.take().is_some() {
            cx.emit(event);
            cx.notify();
        }
    }
}

impl Render for BranchSwitcher {
    /// The picker, centred near the top of the window like Go to File.
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let Some(open) = &self.open else {
            return div().into_any_element();
        };
        let width = px(jig_git::BRANCH_PICKER_WIDTH);
        let left = ((window.viewport_size().width - width) / 2.).max(px(8.));
        deferred(
            anchored()
                .position(point(left, px(TITLE_BAR_HEIGHT + 24.)))
                .snap_to_window_with_margin(px(8.))
                .child(open.view.clone()),
        )
        .into_any_element()
    }
}
