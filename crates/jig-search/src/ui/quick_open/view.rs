//! Drawing Go to File: the query, the rows with their matched letters,
//! and the count of files.

use super::*;

impl QuickOpen {
    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let row = &self.rows[index];
        let selected = index == self.selected;
        let path: &str = row.path.as_ref();
        let name_start = path.rfind('/').map_or(0, |slash| slash + 1);
        let (name, folder) = (&path[name_start..], &path[..name_start.saturating_sub(1)]);

        // Matched characters: accent-coloured, or bold on the accent fill.
        let mark = HighlightStyle {
            color: (!selected).then_some(theme.primary),
            font_weight: Some(FontWeight::BOLD),
            ..Default::default()
        };
        let marks = |range: Range<usize>, shift: usize| -> Vec<(Range<usize>, HighlightStyle)> {
            row.positions
                .iter()
                .filter(|&&i| range.contains(&i))
                .map(|&i| {
                    let len = path[i..].chars().next().map_or(1, char::len_utf8);
                    (i - shift..i - shift + len, mark)
                })
                .collect()
        };
        let name_text = StyledText::new(SharedString::from(name.to_string()))
            .with_highlights(marks(name_start..path.len(), name_start));
        let folder_text = StyledText::new(SharedString::from(folder.to_string()))
            .with_highlights(marks(0..folder.len(), 0));

        let (text, muted) = if selected {
            (
                theme.primary_foreground,
                theme.primary_foreground.opacity(0.75),
            )
        } else {
            (theme.popover_foreground, theme.muted_foreground)
        };
        // Selected like a macOS menu item: accent fill, white text.
        h_flex()
            .id(("quick-open-row", index))
            .w_full()
            .overflow_hidden()
            .h(px(ROW_HEIGHT))
            .px_2p5()
            .gap_2p5()
            .rounded(px(7.))
            .text_color(text)
            .when(selected, |row| row.bg(theme.primary))
            .when(!selected, |row| {
                row.hover(|row| row.bg(theme.foreground.opacity(0.06)))
            })
            .child(
                Icon::default()
                    .data(FILE)
                    .size(px(16.))
                    .flex_none()
                    .text_color(if selected {
                        text
                    } else {
                        theme.primary.opacity(0.85)
                    }),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .items_baseline()
                    .child(
                        div()
                            .flex_none()
                            .max_w(relative(0.7))
                            .truncate()
                            .text_size(px(13.5))
                            .child(name_text),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(muted)
                            .child(folder_text),
                    ),
            )
            .when(selected, |this| {
                this.child(
                    div()
                        .flex_none()
                        .text_size(px(12.))
                        .text_color(muted)
                        .child("↩"),
                )
            })
            .when(row.recent && !selected, |this| {
                this.child(
                    Icon::default()
                        .data(CLOCK)
                        .size(px(12.))
                        .flex_none()
                        .text_color(muted.opacity(0.8)),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    this.choose(index, cx);
                }),
            )
            .into_any_element()
    }

    fn status(&self) -> String {
        let query = self.filtered_query.trim();
        if self.indexing && self.files.is_empty() {
            return "Indexing…".into();
        }
        let count = |n: usize, one: &str, many: &str| {
            let word = if n == 1 { one } else { many };
            let n = if n >= MAX_ROWS && !query.is_empty() {
                format!("{MAX_ROWS}+")
            } else {
                group_digits(n)
            };
            format!("{n} {word}")
        };
        if query.is_empty() {
            count(self.files.len(), "file", "files")
        } else {
            count(self.rows.len(), "match", "matches")
        }
    }
}

impl Render for QuickOpen {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = self.rows.len();
        let visible = rows.min(MAX_VISIBLE_ROWS);
        let empty_query = self.filtered_query.trim().is_empty();
        let section = if empty_query && !self.recent.is_empty() {
            Some("Recently opened")
        } else {
            None
        };

        let list = uniform_list(
            "quick-open-rows",
            rows,
            cx.processor(|this, range: Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .w_full()
        .h(px(visible as f32 * ROW_HEIGHT));

        let empty = (rows == 0).then(|| {
            let message = if self.indexing && self.files.is_empty() {
                "Looking through the project…".to_string()
            } else if empty_query {
                "This project has no files yet.".to_string()
            } else {
                format!("No files match “{}”", self.filtered_query.trim())
            };
            v_flex()
                .items_center()
                .justify_center()
                .py_6()
                .gap_1()
                .child(
                    Icon::default()
                        .data(SEARCH)
                        .size(px(20.))
                        .text_color(theme.muted_foreground.opacity(0.6)),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme.muted_foreground)
                        .child(message),
                )
        });

        let divider = || {
            div()
                .h(px(1.))
                .mx_neg_1p5()
                .bg(theme.foreground.opacity(0.08))
        };

        let panel = jig_commands::surface::panel(cx)
            .flex()
            .flex_col()
            .key_context("JigQuickOpen")
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_up))
            .capture_action(cx.listener(Self::on_down))
            .w(px(WIDTH))
            .p_1p5()
            .child(
                // Spotlight-sized search field.
                h_flex()
                    .h(px(40.))
                    .pl_2p5()
                    .pr_1()
                    .gap_1()
                    .child(
                        Icon::default()
                            .data(SEARCH)
                            .size(px(17.))
                            .flex_none()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(16.))
                            .child(Input::new(&self.input).appearance(false).cleanable(false)),
                    ),
            )
            .child(divider().mb_1())
            .when_some(section, |this, section| {
                this.child(
                    div()
                        .px_2p5()
                        .pt_1()
                        .pb_1()
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.muted_foreground)
                        .child(section),
                )
            })
            .children(empty)
            .when(rows > 0, |this| this.child(list))
            .child(divider().mt_1())
            .child(
                h_flex()
                    .px_2p5()
                    .pt_1p5()
                    .pb_0p5()
                    .justify_between()
                    .child(jig_commands::surface::hint(self.status(), cx))
                    .child(jig_commands::surface::hint(
                        "↑↓ navigate · ↩ open · esc close",
                        cx,
                    )),
            );
        jig_commands::motion::pop_in(panel, "jig-quick-open")
    }
}
