//! The file tree sidebar: which project it shows, showing and hiding it, and
//! opening the files chosen in it.

use std::path::Path;

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_editor::EditorHandle;

use super::tabs::canonical;
use super::{
    FocusFileTree, MAX_SIDEBAR_WIDTH, MIN_SIDEBAR_WIDTH, ProjectTree, ToggleSidebar, Workspace,
};
use crate::file_tree::{FileTree, FileTreeEvent};

impl Workspace {
    /// Show `dir` in the sidebar, opened, as the project.
    pub(super) fn open_folder(&mut self, dir: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.set_tree_root(dir, window, cx);
        self.sidebar_open = true;
        match self.document().path.clone() {
            Some(file) => self.show_in_tree(&file, window, cx),
            // Nothing to edit yet, so start in the tree.
            None => self.focus_tree(window, cx),
        }
        cx.notify();
    }

    /// Highlight `file` in the tree. A file outside the current project
    /// switches the tree to that file's project.
    pub(super) fn show_in_tree(
        &mut self,
        file: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let file = canonical(file);
        let inside = self
            .tree
            .as_ref()
            .is_some_and(|tree| file.starts_with(tree.view.read(cx).root()));
        if !inside {
            self.set_tree_root(&crate::project::root_for(&file), window, cx);
        }
        if let Some(tree) = &self.tree {
            tree.view
                .update(cx, |tree, cx| tree.set_active(Some(&file), cx));
        }
    }

    fn set_tree_root(&mut self, root: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.new(|cx| FileTree::new(root, cx));
        let events = cx.subscribe_in(&view, window, |this, _, event, window, cx| match event {
            FileTreeEvent::Open(path) => this.open_file(path, window, cx),
            FileTreeEvent::Dismissed => this.editor().focus(window, cx),
        });
        self.tree = Some(ProjectTree {
            view,
            _events: events,
        });
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

    fn focus_tree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            self.editor().focus(window, cx);
        }
        self.sidebar_open = !self.sidebar_open;
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
            self.editor().focus(window, cx);
        } else {
            self.sidebar_open = true;
            self.focus_tree(window, cx);
        }
        cx.notify();
    }

    pub(super) fn sidebar_shown(&self) -> bool {
        self.sidebar_open && self.tree.is_some()
    }

    /// The sidebar below the title bar: the project's files.
    pub(super) fn render_sidebar(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let tree = self.tree.as_ref().filter(|_| self.sidebar_open)?;
        Some(
            sidebar_panel(self.sidebar_width, cx)
                .h_full()
                .child(tree.view.clone())
                .into_any_element(),
        )
    }

    /// The sidebar's part of the title bar, under the traffic lights, so
    /// the sidebar reads as running the full height of the window.
    pub(super) fn render_sidebar_top(&self, cx: &Context<Self>) -> Option<AnyElement> {
        self.sidebar_shown().then(|| {
            sidebar_panel(self.sidebar_width, cx)
                .h_full()
                .into_any_element()
        })
    }

    /// The sidebar's draggable edge, over the full height of the window.
    pub(super) fn render_sidebar_handle(&self, cx: &Context<Self>) -> Option<AnyElement> {
        self.sidebar_shown().then(|| {
            div()
                .id("sidebar-resize")
                .absolute()
                .top_0()
                .bottom_0()
                .left(self.sidebar_width - px(3.))
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
                        this.sidebar_width = event
                            .position
                            .x
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
