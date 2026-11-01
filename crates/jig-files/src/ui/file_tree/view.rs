//! Drawing the tree: its header, rows, name field and drag preview.

use std::ops::Range;

use super::*;

/// A row being dragged to another folder.
pub(super) struct DraggedEntry {
    path: PathBuf,
}

/// What follows the pointer while rows are dragged: their name, or how
/// many there are.
pub(super) struct DragPreview {
    label: SharedString,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_2()
            .py_0p5()
            .rounded(px(5.))
            .text_size(px(12.))
            .bg(theme.sidebar_primary)
            .text_color(theme.sidebar_primary_foreground)
            .shadow_sm()
            .child(self.label.clone())
    }
}

pub(super) fn header_button(
    id: &'static str,
    icon: &'static [u8],
    tooltip: &'static str,
    cx: &Context<FileTree>,
    on_click: impl Fn(&mut FileTree, &mut Window, &mut Context<FileTree>) + 'static,
) -> AnyElement {
    let theme = cx.theme();
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(px(22.))
        .rounded(px(5.))
        .text_color(theme.muted_foreground)
        .hover(|s| s.bg(theme.sidebar_accent).text_color(theme.foreground))
        .child(Icon::default().data(icon).size(px(14.)))
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .on_click(cx.listener(move |this, _, window, cx| {
            cx.stop_propagation();
            on_click(this, window, cx)
        }))
        .into_any_element()
}

impl Render for FileTree {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        let root_name = self
            .model
            .root()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let naming_at = self.naming_index();
        let inserts = self.naming_inserts();
        let rows = self.model.rows().len() + usize::from(naming_at.is_some() && inserts);
        let go_to_file = header_button(
            "go-to-file",
            SEARCH,
            "Go to File (⌘P)",
            cx,
            |_, window, cx| window.dispatch_action(Box::new(jig_search::quick_open::GoToFile), cx),
        );
        let new_file = header_button("new-file", FILE_PLUS, "New File", cx, |this, window, cx| {
            this.start_new(NewEntry::File, window, cx)
        });
        let new_folder = header_button(
            "new-folder",
            FOLDER_PLUS,
            "New Folder",
            cx,
            |this, window, cx| this.start_new(NewEntry::Folder, window, cx),
        );
        let theme = cx.theme();
        let error = self.naming.as_ref().and_then(|n| n.error.clone());
        let list =
            uniform_list(
                "file-tree-rows",
                rows,
                cx.processor(move |this, range: Range<usize>, _, cx| {
                    let languages = jig_settings::get(cx).languages;
                    let theme = cx.theme();
                    range
                        .map(|list_ix| {
                            if Some(list_ix) == naming_at
                                && let Some(naming) = &this.naming
                            {
                                let depth = if inserts {
                                    this.naming_depth()
                                } else {
                                    this.model.rows()[list_ix].depth
                                };
                                return h_flex()
                                    .id("new-entry")
                                    .h(px(ROW_HEIGHT))
                                    .w_full()
                                    .pl(px(4. + depth as f32 * INDENT))
                                    .pr_1()
                                    .gap_1()
                                    .child(div().w(px(12.)).flex_none())
                                    .child(div().w(px(15.)).flex_none())
                                    .child(div().flex_1().min_w_0().child(
                                        Input::new(&naming.input).xsmall().cleanable(false),
                                    ));
                            }
                            let ix = match naming_at {
                                Some(at) if inserts && list_ix > at => list_ix - 1,
                                _ => list_ix,
                            };
                            let row = &this.model.rows()[ix];
                            let drop_fill = theme.sidebar_primary.opacity(0.25);
                            let selected = this.is_selected(ix);
                            let active = this.active.as_ref() == Some(&row.path);
                            // Like a Finder sidebar: the accent fill marks the
                            // keyboard selection while the tree has focus, a
                            // quiet grey one the open file.
                            let highlighted = selected && focused;
                            let color = if highlighted {
                                theme.sidebar_primary_foreground
                            } else if row.ignored {
                                theme.muted_foreground.opacity(0.6)
                            } else if active {
                                theme.sidebar_accent_foreground
                            } else {
                                theme.sidebar_foreground
                            };
                            let icon_color = if highlighted {
                                theme.sidebar_primary_foreground
                            } else if row.ignored {
                                theme.muted_foreground.opacity(0.5)
                            } else if row.is_dir {
                                theme.primary
                            } else {
                                theme.muted_foreground
                            };
                            let chevron = row.is_dir.then(|| {
                                Icon::default()
                                    .data(if row.expanded {
                                        CHEVRON_DOWN
                                    } else {
                                        CHEVRON_RIGHT
                                    })
                                    .size(px(10.))
                                    .text_color(if highlighted {
                                        theme.sidebar_primary_foreground
                                    } else {
                                        theme.muted_foreground
                                    })
                            });
                            h_flex()
                                .id(list_ix)
                                .h(px(ROW_HEIGHT))
                                .w_full()
                                .pl(px(4. + row.depth as f32 * INDENT))
                                .pr_2()
                                .gap_1()
                                .rounded(px(6.))
                                .text_size(px(13.))
                                .text_color(color)
                                .when(active && !highlighted, |this| this.bg(theme.sidebar_accent))
                                .when(highlighted, |this| this.bg(theme.sidebar_primary))
                                .when(!highlighted && !active, |this| {
                                    this.hover(|s| s.bg(theme.sidebar_accent.opacity(0.5)))
                                })
                                .child(
                                    div()
                                        .w(px(12.))
                                        .flex_none()
                                        .flex()
                                        .justify_center()
                                        .children(chevron),
                                )
                                .child({
                                    let logo = (!row.is_dir)
                                        .then(|| {
                                            crate::file_icons::for_path(
                                                &row.path,
                                                &languages,
                                                theme.is_dark(),
                                            )
                                        })
                                        .flatten();
                                    let (data, icon_color) = match logo {
                                        // The selection's fill keeps every icon white.
                                        Some((svg, _)) if highlighted || row.ignored => {
                                            (svg, icon_color)
                                        }
                                        Some((svg, color)) => (svg, color),
                                        None if row.is_dir => (FOLDER, icon_color),
                                        None => (FILE, icon_color),
                                    };
                                    // Logos fill their square, so they're drawn a
                                    // little smaller than the outlined file.
                                    let size = if logo.is_some() { 13. } else { 15. };
                                    div()
                                        .size(px(15.))
                                        .flex_none()
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .child(
                                            Icon::default()
                                                .data(data)
                                                .size(px(size))
                                                .text_color(icon_color),
                                        )
                                })
                                .child(div().pl_0p5().truncate().child(row.name.clone()))
                                // Select on press, as Finder does. Pressing focuses
                                // the tree, which would otherwise light up the
                                // previous selection until the click completes.
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                                        this.press_row(ix, event.modifiers, cx)
                                    }),
                                )
                                .on_click(cx.listener(move |this, event, window, cx| {
                                    this.click_row(ix, event, window, cx)
                                }))
                                .on_drag(
                                    DraggedEntry {
                                        path: row.path.clone(),
                                    },
                                    {
                                        let tree = cx.entity().downgrade();
                                        move |dragged, _, _, cx| {
                                            let label = tree
                                                .upgrade()
                                                .map(|tree| tree.read(cx).drag_label(&dragged.path))
                                                .unwrap_or_default();
                                            cx.new(|_| DragPreview { label })
                                        }
                                    },
                                )
                                // Dropped on a folder, rows go into it; on a
                                // file, beside it.
                                .when(row.is_dir, |this| {
                                    this.drag_over::<DraggedEntry>(move |style, _, _, _| {
                                        style.bg(drop_fill)
                                    })
                                })
                                .on_drop(cx.listener({
                                    let dir = if row.is_dir {
                                        row.path.clone()
                                    } else {
                                        row.path.parent().map(Path::to_path_buf).unwrap_or_default()
                                    };
                                    move |this, dragged: &DraggedEntry, window, cx| {
                                        this.move_into(&dragged.path, &dir, window, cx)
                                    }
                                }))
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                        cx.stop_propagation();
                                        this.show_menu(Some(ix), event.position, window, cx);
                                    }),
                                )
                        })
                        .collect()
                }),
            )
            .track_scroll(&self.scroll_handle)
            .size_full()
            .px_2();

        div()
            .id("file-tree")
            .key_context(if self.naming.is_some() {
                NAMING_CONTEXT
            } else {
                CONTEXT
            })
            .track_focus(&self.focus_handle)
            .capture_action(cx.listener(Self::on_naming_escape))
            .on_action(cx.listener(Self::cancel_naming))
            .on_action(cx.listener(Self::select_previous))
            .on_action(cx.listener(Self::select_next))
            .on_action(cx.listener(Self::extend_previous))
            .on_action(cx.listener(Self::extend_next))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::collapse_or_parent))
            .on_action(cx.listener(Self::expand_or_child))
            .on_action(cx.listener(Self::open_selected))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::new_file_here))
            .on_action(cx.listener(Self::new_folder_here))
            .on_action(cx.listener(Self::rename_selected))
            .on_action(cx.listener(Self::trash_selected))
            .on_action(cx.listener(Self::copy_path))
            .on_action(cx.listener(Self::copy_relative_path))
            .on_action(cx.listener(Self::reveal_selected))
            .flex()
            .flex_col()
            .size_full()
            .child(
                h_flex()
                    .pl_4()
                    .pr_2()
                    .pt_0p5()
                    .pb_1()
                    .gap_0p5()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(11.))
                            .font_semibold()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(root_name),
                    )
                    .child(go_to_file)
                    .child(new_file)
                    .child(new_folder),
            )
            .when_some(error, |this, error| {
                this.child(
                    div()
                        .mx_2()
                        .mb_1()
                        .px_2()
                        .py_1()
                        .rounded(px(4.))
                        .text_xs()
                        .text_color(theme.danger)
                        .bg(theme.danger.opacity(0.1))
                        .child(error),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    // Rows stop the drop and the press, so these are for
                    // the space below them: the project folder.
                    .on_drop(cx.listener(|this, dragged: &DraggedEntry, window, cx| {
                        let root = this.model.root().to_path_buf();
                        this.move_into(&dragged.path, &root, window, cx)
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            this.show_menu(None, event.position, window, cx)
                        }),
                    )
                    .child(list)
                    .vertical_scrollbar(&self.scroll_handle),
            )
    }
}
