//! Drawing the Git view: its header, commit box and file rows.

use super::*;

impl GitPanel {
    /// The view's title and how far the branch is from its upstream, with
    /// the remote actions on a strip below.
    fn render_header(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (ahead, behind) = self.status.as_ref().map_or((0, 0), |s| (s.ahead, s.behind));
        let counts = [(ahead, "↑"), (behind, "↓")]
            .into_iter()
            .filter(|(n, _)| *n > 0)
            .map(|(n, arrow)| format!("{arrow}{n}"))
            .collect::<Vec<_>>()
            .join(" ");
        let action = |id: &'static str, icon: &'static [u8], label: &'static str| {
            h_flex()
                .id(id)
                .debug_selector(move || id.into())
                .flex_1()
                .justify_center()
                .h(px(22.))
                .gap_1()
                .rounded(px(5.))
                .text_size(px(11.5))
                .text_color(theme.muted_foreground)
                .hover(|s| s.bg(theme.sidebar_accent).text_color(theme.foreground))
                .child(Icon::default().data(icon).size(px(12.)).flex_none())
                .child(label)
        };
        let title = h_flex()
            .h(px(22.))
            .px_2()
            .gap_1p5()
            .text_size(px(11.))
            .font_semibold()
            .text_color(theme.muted_foreground)
            .child(div().truncate().child("Changes"))
            .when(!counts.is_empty(), |this| {
                this.child(
                    div()
                        .flex_none()
                        .font_weight(FontWeight::NORMAL)
                        .child(counts),
                )
            });
        let actions = h_flex()
            .p_0p5()
            .gap_0p5()
            .rounded(px(7.))
            .border_1()
            .border_color(theme.foreground.opacity(0.08))
            .bg(theme.foreground.opacity(0.03))
            .child(
                action("git-fetch", FETCH, "Fetch")
                    .on_click(cx.listener(|this, _, window, cx| this.fetch(window, cx))),
            )
            .child(
                action("git-pull", PULL, "Pull")
                    .on_click(cx.listener(|this, _, window, cx| this.pull(window, cx))),
            )
            .child(
                action("git-push", PUSH, "Push")
                    .on_click(cx.listener(|this, _, window, cx| this.push(window, cx))),
            )
            .child(
                action("git-branches", BRANCH_ICON, "Branch")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(GitPanelEvent::Branches))),
            );
        v_flex()
            .pt_0p5()
            .gap_1()
            .child(title)
            .child(actions)
            .into_any_element()
    }

    fn render_commit(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let count = self.files().iter().filter(|f| self.is_checked(f)).count();
        let label = match count {
            0 => "Commit".to_string(),
            n => format!("Commit {n}"),
        };
        v_flex()
            .gap_1p5()
            .child(
                div()
                    .w_full()
                    .h(px(30.))
                    .px_2()
                    .flex()
                    .items_center()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(theme.foreground.opacity(0.12))
                    .bg(theme.foreground.opacity(0.03))
                    .text_size(px(13.))
                    .child(
                        Input::new(&self.message)
                            .w_full()
                            .appearance(false)
                            .cleanable(false),
                    ),
            )
            .child(
                div()
                    .id("git-commit")
                    .debug_selector(|| "git-commit".into())
                    .w_full()
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .bg(theme.primary)
                    .text_color(theme.primary_foreground)
                    .text_size(px(12.5))
                    .font_weight(FontWeight::MEDIUM)
                    .hover(|s| s.opacity(0.9))
                    .child(label)
                    .on_click(cx.listener(|this, _, window, cx| this.commit(window, cx))),
            )
            .into_any_element()
    }

    fn render_row(&self, ix: usize, cx: &Context<Self>) -> AnyElement {
        match self.rows[ix] {
            Row::Header { staged, count } => self.render_header_row(staged, count, cx),
            Row::File { staged, index } => self.render_file(staged, ix, index, cx),
        }
    }

    fn render_header_row(&self, staged: bool, count: usize, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let title = if staged { "Staged" } else { "Changes" };
        let checked = self.checked_in(staged).len();
        let action = match (checked, staged) {
            (0, _) => None,
            (n, true) => Some(format!("Unstage {n}")),
            (n, false) => Some(format!("Stage {n}")),
        };
        h_flex()
            .h(px(ROW_HEIGHT))
            .px_2()
            .gap_2()
            .text_size(px(11.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.muted_foreground)
            .child(
                checkbox(self.section_check(staged), cx)
                    .id(if staged {
                        "git-check-staged"
                    } else {
                        "git-check-unstaged"
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_section(staged, cx))),
            )
            .child(format!("{title}  {count}"))
            .child(div().flex_1())
            .children(action.map(|label| {
                div()
                    .id(if staged {
                        "git-unstage-checked"
                    } else {
                        "git-stage-checked"
                    })
                    .px_1p5()
                    .rounded(px(4.))
                    .hover(|s| s.bg(theme.foreground.opacity(0.08)))
                    .child(label)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.stage_checked(staged, window, cx)
                    }))
            }))
            .into_any_element()
    }

    fn render_file(&self, staged: bool, ix: usize, index: usize, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(file) = self.status.as_ref().and_then(|s| s.files.get(index)) else {
            return div().h(px(ROW_HEIGHT)).into_any_element();
        };
        let change = if staged { file.staged } else { file.unstaged };
        let change = change.unwrap_or(Change::Modified);
        let color = match change {
            Change::Added | Change::Untracked => theme.green,
            Change::Deleted | Change::Conflicted => theme.red,
            Change::Modified | Change::Renamed => theme.blue,
        };
        // An untracked folder comes as one entry, ending in `/`.
        let folder = file.path.ends_with('/');
        let path = file.path.trim_end_matches('/');
        let (dir, name) = match path.rsplit_once('/') {
            Some((dir, name)) => (dir.to_string(), name.to_string()),
            None => (String::new(), path.to_string()),
        };
        let name = if folder { format!("{name}/") } else { name };
        let group = SharedString::from(format!("git-file-{ix}"));
        let path = file.path.clone();
        let check = if self.is_checked(file) {
            Check::On
        } else {
            Check::Off
        };
        let full = self.repo.root().join(&file.path);
        h_flex()
            .id(("git-file", ix))
            .group(group.clone())
            .h(px(ROW_HEIGHT))
            .px_2()
            .gap_2()
            .rounded(px(6.))
            .hover(|s| s.bg(theme.foreground.opacity(0.06)))
            .child(checkbox(check, cx).id(("git-check", ix)).on_click({
                let path = path.clone();
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.toggle(&path, cx);
                })
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(12.))
                    .text_size(px(11.))
                    .font_weight(FontWeight::BOLD)
                    .font_family(theme.mono_font_family.clone())
                    .text_color(color)
                    .child(change.letter()),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1p5()
                    .items_baseline()
                    .child(
                        div()
                            .flex_shrink(1.)
                            .min_w(px(40.))
                            .truncate()
                            .text_size(px(13.))
                            .when(change == Change::Deleted, |this| this.line_through())
                            .child(name),
                    )
                    .when(!dir.is_empty(), |this| {
                        this.child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(11.5))
                                .text_color(theme.muted_foreground)
                                .child(dir),
                        )
                    }),
            )
            .child(
                div()
                    .id(("git-stage", ix))
                    .flex_none()
                    .size(px(20.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(5.))
                    .invisible()
                    .group_hover(group, |s| s.visible())
                    .hover(|s| s.bg(theme.foreground.opacity(0.1)))
                    .child(
                        Icon::default()
                            .data(if staged { MINUS } else { PLUS })
                            .size(px(12.))
                            .text_color(theme.foreground.opacity(0.8)),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        if staged {
                            this.unstage(vec![path.clone()], window, cx)
                        } else {
                            this.stage(vec![path.clone()], window, cx)
                        }
                    })),
            )
            .when(change != Change::Deleted && !folder, |row| {
                row.on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(GitPanelEvent::Open(full.clone()))),
                )
            })
            .into_any_element()
    }
}

impl Render for GitPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let clean = self.status.is_some() && self.rows.is_empty();
        let rows = self.rows.len();
        let list = uniform_list(
            "git-files",
            rows,
            cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .size_full();
        let header = self.render_header(cx);
        let commit = self.render_commit(cx);
        let theme = cx.theme();
        let footer = match (&self.error, self.busy) {
            (_, Some(busy)) => div()
                .text_size(px(11.5))
                .text_color(theme.muted_foreground)
                .child(busy),
            (Some(error), None) => div()
                .text_size(px(11.5))
                .text_color(theme.danger)
                .child(error.clone()),
            (None, None) => jig_commands::surface::hint("↩ commits the checked files", cx),
        };
        v_flex()
            .id("git-panel")
            .key_context("JigGitPanel")
            .capture_action(cx.listener(Self::on_escape))
            .size_full()
            .px_2()
            .gap_1p5()
            .child(header)
            .child(commit)
            // The list fills the space, so the empty message takes its place
            // rather than following it.
            .child(div().flex_1().min_h_0().map(|this| {
                if clean {
                    this.child(
                        div()
                            .px_2()
                            .py_3()
                            .text_size(px(13.))
                            .text_color(theme.muted_foreground)
                            .child("No changes since the last commit."),
                    )
                } else {
                    this.child(list)
                }
            }))
            .child(div().px_1().pb_2().child(footer))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Check {
    On,
    Off,
    /// Some of a section's files.
    Some,
}

/// A small macOS-style checkbox.
pub(super) fn checkbox(check: Check, cx: &App) -> Div {
    let theme = cx.theme();
    let filled = check != Check::Off;
    div()
        .flex_none()
        .size(px(14.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .border_1()
        .border_color(if filled {
            theme.primary
        } else {
            theme.foreground.opacity(0.25)
        })
        .when(filled, |this| this.bg(theme.primary))
        .when(!filled, |this| this.bg(theme.foreground.opacity(0.03)))
        .when(filled, |this| {
            this.child(
                Icon::default()
                    .data(if check == Check::On { CHECK } else { MINUS })
                    .size(px(10.))
                    .text_color(theme.primary_foreground),
            )
        })
}
