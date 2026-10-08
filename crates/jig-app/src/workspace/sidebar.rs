//! The sidebar: a narrow activity bar along the window's left edge picks
//! what it shows, the project's files or Git, and collapses it. Also which
//! project the tree shows, and opening the files chosen in it.

use std::path::Path;
use std::time::{Duration, Instant};

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::tabs::canonical;
use super::{
    FocusFileTree, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, ProjectTree, TITLE_BAR_HEIGHT,
    ToggleSidebar, Workspace,
};
use crate::file_tree::{FileTree, FileTreeEvent};
use crate::git_panel::BRANCH_ICON;
use crate::settings_window::OpenSettings;

/// The activity bar's width.
pub(super) const ACTIVITY_BAR_WIDTH: f32 = 44.;

/// Lucide "files".
const FILES_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 7h-3a2 2 0 0 1-2-2V2"/><path d="M9 18a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h7l4 4v10a2 2 0 0 1-2 2Z"/><path d="M3 7.6v12.8A1.6 1.6 0 0 0 4.6 22h9.8"/></svg>"#;

/// Lucide "settings".
const SETTINGS_ICON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle cx="12" cy="12" r="3"/></svg>"#;

/// How long the sidebar takes to fold out or collapse.
const MOTION: Duration = Duration::from_millis(180);

/// The sidebar folding out or collapsing: how open it was when the motion
/// started, so a toggle midway turns it around from where it is.
pub(super) struct SidebarMotion {
    started: Instant,
    from: f32,
}

/// What the sidebar shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum SidebarView {
    #[default]
    Files,
    Git,
}

impl Workspace {
    /// Show `dir` in the sidebar, opened, as the project.
    pub(super) fn open_folder(&mut self, dir: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let empty = self.tabs.len() == 1 && self.tab().is_blank(cx);
        if empty {
            // Nothing worth keeping on screen: show the project's page.
            self.home = true;
        }
        self.set_tree_root(dir, window, cx);
        self.set_sidebar_open(true);
        if empty {
            self.restore_session(dir, window, cx);
        }
        match self.document().path.clone() {
            Some(file) => self.show_in_tree(&file, window, cx),
            // Nothing to edit yet, so start in the tree.
            None => self.focus_tree(window, cx),
        }
        cx.notify();
    }

    /// Highlight `file` in the tree. The first file opened picks the project;
    /// after that the tree stays put, and a file from elsewhere (a settings
    /// file, say) just leaves nothing highlighted.
    pub(super) fn show_in_tree(
        &mut self,
        file: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let file = canonical(file);
        if self.tree.is_none() {
            self.set_tree_root(&crate::project::root_for(&file), window, cx);
        }
        if let Some(tree) = &self.tree {
            tree.view.update(cx, |tree, cx| {
                let inside = file.starts_with(tree.root());
                tree.set_active(inside.then_some(file.as_path()), cx)
            });
        }
    }

    fn set_tree_root(&mut self, root: &Path, window: &mut Window, cx: &mut Context<Self>) {
        crate::recent::update(cx, |recent| recent.push(&canonical(root)));
        let view = cx.new(|cx| FileTree::new(root, cx));
        let events = cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
            FileTreeEvent::Open(path) => this.open_file(path, window, cx),
            FileTreeEvent::Dismissed => this.focus_main(window, cx),
            FileTreeEvent::Renamed { from, to } => this.follow_rename(from, to, window, cx),
            FileTreeEvent::Trashed(path) => this.close_trashed(path, window, cx),
        });
        self.tree = Some(ProjectTree {
            view,
            _events: events,
        });
        self.refresh_git(window, cx);
    }

    pub(super) fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        if let Some(tree) = &self.tree {
            tree.view.update(cx, |tree, cx| tree.refresh(cx));
        }
    }

    fn tree_focused(&self, window: &Window, cx: &App) -> bool {
        self.tree
            .as_ref()
            .is_some_and(|tree| tree.view.read(cx).is_focused(window))
    }

    pub(super) fn focus_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tree) = &self.tree {
            tree.view.update(cx, |tree, cx| tree.focus(window, cx));
        }
    }

    pub(super) fn toggle_sidebar(
        &mut self,
        _: &ToggleSidebar,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tree.is_none() {
            return;
        }
        if self.sidebar_open && self.tree_focused(window, cx) {
            self.focus_main(window, cx);
        }
        self.set_sidebar_open(!self.sidebar_open);
        self.sync_git_watch(window, cx);
        cx.notify();
    }

    /// Move focus into the tree, opening the sidebar if needed, or back to
    /// the editor when the tree already has it.
    pub(super) fn focus_file_tree(
        &mut self,
        _: &FocusFileTree,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.tree.is_none() {
            return;
        }
        if self.tree_focused(window, cx) {
            self.focus_main(window, cx);
        } else {
            self.set_sidebar_open(true);
            self.sidebar_view = SidebarView::Files;
            self.focus_tree(window, cx);
        }
        self.sync_git_watch(window, cx);
        cx.notify();
    }

    /// An activity bar button: show `view`, or collapse the sidebar when it
    /// already shows it.
    pub(super) fn toggle_sidebar_view(
        &mut self,
        view: SidebarView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.sidebar_shown() && self.sidebar_view == view {
            self.set_sidebar_open(false);
            self.focus_main(window, cx);
        } else {
            self.set_sidebar_open(true);
            self.sidebar_view = view;
            match view {
                SidebarView::Files => self.focus_tree(window, cx),
                SidebarView::Git => self.show_git_view(window, cx),
            }
        }
        self.sync_git_watch(window, cx);
        cx.notify();
    }

    /// Open or collapse the sidebar, animating from wherever it is now.
    fn set_sidebar_open(&mut self, open: bool) {
        if open == self.sidebar_open {
            return;
        }
        self.sidebar_motion = Some(SidebarMotion {
            started: Instant::now(),
            from: self.sidebar_openness(),
        });
        self.sidebar_open = open;
    }

    /// How far the sidebar is folded out, from 0 (collapsed) to 1 (open).
    fn sidebar_openness(&self) -> f32 {
        let target = if self.sidebar_open { 1. } else { 0. };
        let Some(motion) = &self.sidebar_motion else {
            return target;
        };
        let t = (motion.started.elapsed().as_secs_f32() / MOTION.as_secs_f32()).min(1.);
        let eased = ease_out_quint()(t);
        motion.from + (target - motion.from) * eased
    }

    /// Call each frame: ends a finished motion, or asks for the next frame.
    pub(super) fn step_sidebar_motion(&mut self, window: &Window) {
        if let Some(motion) = &self.sidebar_motion {
            if motion.started.elapsed() >= MOTION {
                self.sidebar_motion = None;
            } else {
                window.request_animation_frame();
            }
        }
    }

    /// The activity bar is there whenever there's a project to show.
    pub(super) fn activity_bar_shown(&self) -> bool {
        self.tree.is_some()
    }

    /// How far from the window's left edge the editor starts.
    fn sidebar_right_edge(&self) -> Pixels {
        let bar = if self.activity_bar_shown() {
            px(ACTIVITY_BAR_WIDTH)
        } else {
            px(0.)
        };
        if self.sidebar_shown() {
            bar + self.sidebar_width
        } else {
            bar
        }
    }

    pub(super) fn sidebar_shown(&self) -> bool {
        self.sidebar_open && self.tree.is_some()
    }

    /// The sidebar below the title bar: the project's files, or Git. While
    /// it folds out or collapses, its content keeps its full width and is
    /// clipped, so the text slides rather than rewrapping.
    pub(super) fn render_sidebar(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let tree = self.tree.as_ref()?;
        let openness = self.sidebar_openness();
        if openness <= 0. {
            return None;
        }
        let content = match self.sidebar_view {
            SidebarView::Files => tree.view.clone().into_any_element(),
            SidebarView::Git => self.render_git_view(cx),
        };
        Some(
            div()
                .flex_none()
                .h_full()
                .w(self.sidebar_width * openness)
                .overflow_hidden()
                .child(
                    sidebar_panel(self.sidebar_width, cx)
                        .h_full()
                        .child(content),
                )
                .into_any_element(),
        )
    }

    /// The strip along the window's left edge: a button per sidebar view.
    pub(super) fn render_activity_bar(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if !self.activity_bar_shown() {
            return None;
        }
        let button = |id: &'static str, icon: &'static [u8], tooltip: &'static str, view| {
            let active = self.sidebar_shown() && self.sidebar_view == view;
            activity_button(id, icon, tooltip, active, cx).on_click(
                cx.listener(move |this, _, window, cx| this.toggle_sidebar_view(view, window, cx)),
            )
        };
        Some(
            sidebar_panel(px(ACTIVITY_BAR_WIDTH), cx)
                .h_full()
                .flex()
                .flex_col()
                .items_center()
                .py_1()
                .gap_1()
                .child(button(
                    "activity-files",
                    FILES_ICON,
                    "Files (⇧⌘E)",
                    SidebarView::Files,
                ))
                .child(button(
                    "activity-git",
                    BRANCH_ICON,
                    "Git (⌃⇧G)",
                    SidebarView::Git,
                ))
                .child(div().flex_1())
                .child(settings_button(cx))
                .into_any_element(),
        )
    }

    /// Settings, floating in the window's bottom-left corner while there's
    /// no activity bar to hold it.
    pub(super) fn render_floating_settings(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.activity_bar_shown() {
            return None;
        }
        // Above the status bar when there is one.
        let bottom = if self.home {
            px(8.)
        } else {
            px(super::status_bar::HEIGHT + 8.)
        };
        Some(
            div()
                .absolute()
                .left(px((ACTIVITY_BAR_WIDTH - 32.) / 2.))
                .bottom(bottom)
                .child(settings_button(cx))
                .into_any_element(),
        )
    }

    /// The sidebar's draggable edge, from the title bar down. Not there
    /// while the sidebar is moving.
    pub(super) fn render_sidebar_handle(&self, cx: &Context<Self>) -> Option<AnyElement> {
        (self.sidebar_shown() && self.sidebar_motion.is_none()).then(|| {
            div()
                .id("sidebar-resize")
                .absolute()
                .top(px(TITLE_BAR_HEIGHT))
                .bottom_0()
                .left(self.sidebar_right_edge() - px(3.))
                .w(px(6.))
                .cursor_col_resize()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.resizing_sidebar = true;
                        cx.stop_propagation();
                    }),
                )
                .into_any_element()
        })
    }

    /// Mouse handlers for the whole window, active while the sidebar's
    /// edge is being dragged.
    pub(super) fn sidebar_drag_handlers(&self, root: Div, cx: &Context<Self>) -> Div {
        root.when(self.resizing_sidebar, |root| {
            root.cursor_col_resize()
                .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                    if !event.dragging() {
                        this.resizing_sidebar = false;
                    } else {
                        let bar = this.sidebar_right_edge() - this.sidebar_width;
                        this.sidebar_width = (event.position.x - bar)
                            .clamp(px(MIN_SIDEBAR_WIDTH), px(MAX_SIDEBAR_WIDTH));
                    }
                    cx.notify();
                }))
                .on_mouse_up(
                    MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.resizing_sidebar = false;
                        cx.notify();
                    }),
                )
        })
    }
}

/// A square icon button in the activity bar's style.
fn activity_button(
    id: &'static str,
    icon: &'static [u8],
    tooltip: &'static str,
    active: bool,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    div()
        .id(id)
        .debug_selector(move || id.into())
        .size(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(7.))
        .text_color(if active {
            theme.foreground
        } else {
            theme.muted_foreground
        })
        .when(active, |this| this.bg(theme.sidebar_accent))
        .hover(|s| s.bg(theme.sidebar_accent).text_color(theme.foreground))
        .child(Icon::default().data(icon).size(px(17.)))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
}

fn settings_button(cx: &App) -> Stateful<Div> {
    activity_button("settings", SETTINGS_ICON, "Settings (⌘,)", false, cx)
        .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx))
}

/// The sidebar's surface. On macOS it is translucent over the window's
/// blur, like Finder's; elsewhere the window is opaque, so it is solid.
fn sidebar_panel(width: Pixels, cx: &App) -> Div {
    let theme = cx.theme();
    let background = if cfg!(target_os = "macos") {
        theme.sidebar.opacity(0.8)
    } else {
        theme.sidebar
    };
    div()
        .relative()
        .flex_none()
        .w(width)
        .bg(background)
        .border_r_1()
        .border_color(theme.sidebar_border)
}
