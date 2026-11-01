//! The tab bar and the window's title.

use super::*;

impl Workspace {
    /// Each tab's label: the file name, plus its folder when another tab
    /// has the same name.
    pub(in crate::workspace) fn tab_labels(&self) -> Vec<String> {
        let titles: Vec<String> = self.tabs.iter().map(|tab| tab.document.title()).collect();
        self.tabs
            .iter()
            .zip(&titles)
            .map(|(tab, title)| {
                let shared = titles.iter().filter(|other| *other == title).count() > 1;
                let parent = tab
                    .document
                    .path
                    .as_deref()
                    .and_then(Path::parent)
                    .and_then(Path::file_name);
                match parent {
                    Some(parent) if shared => format!("{title} — {}", parent.to_string_lossy()),
                    _ => title.clone(),
                }
            })
            .collect()
    }

    /// What the title bar shows: the project's name, or nothing outside a
    /// project. Either way it fills the space, keeping the controls right.
    pub(in crate::workspace) fn render_title(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let title = self.project_root(cx).and_then(|root| {
            root.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        });
        h_flex()
            .flex_1()
            .min_w_0()
            .justify_center()
            .gap_1()
            .text_size(px(13.))
            .children(title.map(|title| {
                div()
                    .truncate()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground.opacity(0.85))
                    .child(title)
            }))
            .into_any_element()
    }

    /// The open tabs, in a row above the editor, even when there's only
    /// one. Only Code shows file tabs.
    pub(in crate::workspace) fn render_tab_bar(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.home || self.mode != super::super::mode::Mode::Code {
            return None;
        }
        let theme = cx.theme();
        let languages = jig_settings::get(cx).languages;
        let workspace = cx.entity().downgrade();
        let count = self.tabs.len();
        let tabs = self
            .tabs
            .iter()
            .zip(self.tab_labels())
            .enumerate()
            .map(|(ix, (tab, label))| {
                let active = ix == self.active;
                let group = SharedString::from(format!("tab-{ix}"));
                // The close button sits on the left, as in Safari, on the
                // hovered tab; an unsaved tab shows a dot there until hovered.
                let close = div()
                    .id(("close-tab", ix))
                    .absolute()
                    .inset_0()
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .hover(|s| s.bg(theme.foreground.opacity(0.1)))
                    .child(
                        Icon::default()
                            .data(CLOSE)
                            .size(px(9.))
                            .text_color(theme.muted_foreground),
                    )
                    .invisible()
                    .group_hover(group.clone(), |s| s.visible())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.close_tab_at(ix, window, cx);
                    }));
                let marker = tab.dirty.then(|| {
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .group_hover(group.clone(), |s| s.invisible())
                        .child(div().size(px(6.)).rounded_full().bg(theme.muted_foreground))
                });
                h_flex()
                    .id(("tab", ix))
                    .group(group.clone())
                    .relative()
                    .flex_1()
                    .flex_basis(px(0.))
                    .min_w(px(MIN_TAB_WIDTH))
                    .max_w(px(MAX_TAB_WIDTH))
                    .h(px(28.))
                    .px_1p5()
                    .gap_1()
                    .rounded(px(7.))
                    .text_size(px(12.))
                    .when(active, |this| {
                        this.bg(theme.foreground.opacity(0.07))
                            .text_color(theme.foreground)
                            .font_weight(FontWeight::MEDIUM)
                    })
                    .when(!active, |this| {
                        this.text_color(theme.muted_foreground)
                            .hover(|s| s.bg(theme.foreground.opacity(0.04)))
                    })
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .size(px(16.))
                            .children(marker)
                            .child(close),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .justify_center()
                            .items_center()
                            .gap_1p5()
                            .children(
                                tab.document
                                    .path
                                    .as_deref()
                                    .and_then(|path| {
                                        jig_files::file_icons::for_path(
                                            path,
                                            &languages,
                                            theme.is_dark(),
                                        )
                                    })
                                    .map(|(svg, color)| {
                                        Icon::default()
                                            .data(svg)
                                            .size(px(11.))
                                            .flex_none()
                                            .text_color(if active {
                                                color
                                            } else {
                                                color.opacity(0.7)
                                            })
                                    }),
                            )
                            .child(div().truncate().child(label)),
                    )
                    // Balances the close button so the name stays centred.
                    .child(div().flex_none().size(px(16.)))
                    // Clicking a tab selects it rather than dragging the window.
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| this.activate(ix, window, cx)))
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(move |this, _, window, cx| this.close_tab_at(ix, window, cx)),
                    )
                    .context_menu({
                        let workspace = workspace.clone();
                        move |menu, _, _| tab_menu(workspace.clone(), ix, count, menu)
                    })
            });
        Some(
            h_flex()
                .id("tab-bar")
                .flex_none()
                .w_full()
                .h(px(TAB_BAR_HEIGHT))
                .px_2()
                .gap_1()
                .border_b_1()
                .border_color(theme.title_bar_border)
                .overflow_x_scroll()
                .track_scroll(&self.tab_scroll)
                .children(tabs)
                .into_any_element(),
        )
    }
}

/// Right-clicking a tab offers to close it, the others, or all of them.
fn tab_menu(
    workspace: WeakEntity<Workspace>,
    ix: usize,
    tabs: usize,
    menu: PopupMenu,
) -> PopupMenu {
    let item = |label: &'static str, keep: Option<Option<usize>>| {
        let workspace = workspace.clone();
        PopupMenuItem::new(label).on_click(move |_, window, cx| {
            workspace
                .update(cx, |this, cx| match keep {
                    None => this.close_tab_at(ix, window, cx),
                    Some(keep) => this.close_tabs_except(keep, window, cx),
                })
                .ok();
        })
    };
    menu.item(item("Close Tab", None))
        .item(item("Close Other Tabs", Some(Some(ix))).disabled(tabs < 2))
        .item(item("Close All Tabs", Some(None)))
}
