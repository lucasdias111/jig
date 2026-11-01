//! Drawing Find in Files: the query and its options, the matching lines
//! and the preview.

use super::*;

impl FindInFiles {
    fn render_row(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let found: &LineMatch = &self.results.matches[index];
        let selected = index == self.selected;
        let name = found
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&found.path)
            .to_string();
        let mark = HighlightStyle {
            background_color: Some(if selected {
                theme.primary_foreground.opacity(0.25)
            } else {
                theme.primary.opacity(0.22)
            }),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        };
        let line = StyledText::new(SharedString::from(found.text.clone()))
            .with_highlights(found.highlights.iter().map(|range| (range.clone(), mark)));
        let (text, muted) = if selected {
            (
                theme.primary_foreground,
                theme.primary_foreground.opacity(0.75),
            )
        } else {
            (theme.popover_foreground, theme.muted_foreground)
        };
        h_flex()
            .id(("find-in-files-row", index))
            // Held to the list's width, so a long line truncates instead of
            // pushing the file name out of view.
            .w_full()
            .overflow_hidden()
            .h(px(ROW_HEIGHT))
            .px_2p5()
            .gap_4()
            .rounded(px(6.))
            .text_color(text)
            .when(selected, |row| row.bg(theme.primary))
            .when(!selected, |row| {
                row.hover(|row| row.bg(theme.foreground.opacity(0.06)))
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(px(12.5))
                    .child(line),
            )
            .child(
                h_flex()
                    .id(("find-in-files-file", index))
                    .flex_none()
                    .flex_shrink_0()
                    .max_w(px(260.))
                    .gap_1p5()
                    .text_size(px(12.))
                    .child(div().min_w_0().truncate().text_color(muted).child(name))
                    .child(
                        div()
                            .flex_none()
                            .text_color(muted.opacity(0.8))
                            .child((found.line + 1).to_string()),
                    )
                    .test_support(),
            )
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 {
                    this.choose(index, cx);
                } else {
                    this.select(index, window, cx);
                    this.input.update(cx, |input, cx| input.focus(window, cx));
                }
            }))
            .test_support()
            .into_any_element()
    }

    /// An option chip, IntelliJ style: `Cc`, `W`, `.*`.
    fn render_option(
        &self,
        id: &'static str,
        label: &'static str,
        tooltip: &'static str,
        on: bool,
        cx: &mut Context<Self>,
        toggle: impl Fn(&mut SearchOptions) + 'static,
    ) -> AnyElement {
        let theme = cx.theme();
        div()
            .id(id)
            .flex()
            .items_center()
            .justify_center()
            .h(px(24.))
            .min_w(px(28.))
            .px_1p5()
            .rounded(px(6.))
            .text_size(px(12.))
            .font_family(theme.mono_font_family.clone())
            .font_weight(FontWeight::SEMIBOLD)
            .cursor_pointer()
            .when(on, |this| {
                this.bg(theme.primary.opacity(0.16))
                    .text_color(theme.primary)
                    .border_1()
                    .border_color(theme.primary.opacity(0.45))
            })
            .when(!on, |this| {
                this.text_color(theme.muted_foreground)
                    .border_1()
                    .border_color(transparent_black())
                    .hover(|s| {
                        s.bg(theme.foreground.opacity(0.06))
                            .text_color(theme.foreground)
                    })
            })
            .child(label)
            .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
            // Keep the search field focused.
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                this.toggle(window, cx, &toggle);
            }))
            .into_any_element()
    }

    fn status(&self) -> String {
        if let Some(error) = &self.error {
            return error.clone();
        }
        if self.searching {
            return "Searching…".into();
        }
        if self.searched.0.is_empty() {
            return match &self.files {
                Some(files) => format!("{} files", group_digits(files.len())),
                None => "Indexing…".into(),
            };
        }
        let lines = self.results.total_lines;
        let files = self.results.files;
        let plural =
            |n: usize, one: &'static str, many: &'static str| if n == 1 { one } else { many };
        let mut status = format!(
            "{} {} in {} {}",
            group_digits(lines),
            plural(lines, "match", "matches"),
            group_digits(files),
            plural(files, "file", "files"),
        );
        if lines > MAX_RESULTS {
            status.push_str(&format!(
                " · showing the first {}",
                group_digits(MAX_RESULTS)
            ));
        }
        status
    }
}

impl Render for FindInFiles {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let options = self.options;
        let match_case = self.render_option(
            "opt-case",
            "Cc",
            "Match Case (⌥C)",
            options.match_case,
            cx,
            |o| o.match_case = !o.match_case,
        );
        let whole_word = self.render_option(
            "opt-word",
            "W",
            "Words (⌥W)",
            options.whole_word,
            cx,
            |o| o.whole_word = !o.whole_word,
        );
        let regex = self.render_option("opt-regex", ".*", "Regex (⌥X)", options.regex, cx, |o| {
            o.regex = !o.regex
        });

        let theme = cx.theme();
        let rows = self.results.matches.len();
        let visible = rows.min(MAX_VISIBLE_ROWS);
        let list = uniform_list(
            "find-in-files-rows",
            rows,
            cx.processor(|this, range: Range<usize>, _, cx| {
                range.map(|ix| this.render_row(ix, cx)).collect::<Vec<_>>()
            }),
        )
        .track_scroll(&self.scroll)
        .w_full()
        .h(px(visible as f32 * ROW_HEIGHT));

        let empty = (rows == 0 && !self.searching && self.error.is_none()).then(|| {
            let query = self.searched.0.clone();
            let message = if query.is_empty() {
                "Type to search every file in the project".to_string()
            } else {
                format!("Nothing found for “{query}”")
            };
            v_flex()
                .items_center()
                .justify_center()
                .py_8()
                .gap_1p5()
                .child(
                    Icon::default()
                        .data(SEARCH)
                        .size(px(22.))
                        .text_color(theme.muted_foreground.opacity(0.5)),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(theme.muted_foreground)
                        .child(message),
                )
        });

        let preview = self.preview.as_ref().filter(|_| rows > 0).map(|preview| {
            let line = self
                .results
                .matches
                .get(self.selected)
                .map_or(0, |found| found.line + 1);
            v_flex()
                .mx_neg_1p5()
                .mb_neg_1p5()
                .border_t_1()
                .border_color(theme.foreground.opacity(0.08))
                .child(
                    h_flex()
                        .px_4()
                        .py_1p5()
                        .gap_1()
                        .text_size(px(11.5))
                        .text_color(theme.muted_foreground)
                        .bg(theme.foreground.opacity(0.03))
                        .child(div().truncate().child(preview.path.clone()))
                        .child(div().flex_none().opacity(0.7).child(format!(":{line}"))),
                )
                .child(
                    div()
                        .h(px(PREVIEW_HEIGHT))
                        .bg(theme.background)
                        .rounded_b(px(jig_commands::surface::RADIUS))
                        .pl_1()
                        .pt_1()
                        .child(
                            gpui_kit::component::input::Editor::new(preview.editor.state())
                                .bordered(false)
                                .size_full(),
                        ),
                )
        });

        let divider = || {
            div()
                .h(px(1.))
                .mx_neg_1p5()
                .bg(theme.foreground.opacity(0.08))
        };
        let status_color = if self.error.is_some() {
            theme.danger
        } else {
            theme.muted_foreground
        };

        let panel = jig_commands::surface::panel(cx)
            .flex()
            .flex_col()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::toggle_match_case))
            .on_action(cx.listener(Self::toggle_whole_word))
            .on_action(cx.listener(Self::toggle_regex))
            .capture_action(cx.listener(Self::on_enter))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_up))
            .capture_action(cx.listener(Self::on_down))
            .w(px(WIDTH))
            .p_1p5()
            .child(
                h_flex()
                    .px_2p5()
                    .pt_1()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .justify_between()
                    .child("Find in Files")
                    .child(
                        div()
                            .font_weight(FontWeight::NORMAL)
                            .text_color(status_color)
                            .child(self.status()),
                    ),
            )
            .child(
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
                    )
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_0p5()
                            .child(match_case)
                            .child(whole_word)
                            .child(regex),
                    ),
            )
            .child(divider().mb_1())
            .children(empty)
            .when(rows > 0, |this| this.child(list).child(div().h_1()))
            .children(preview)
            .when(self.preview.is_none() || rows == 0, |this| {
                this.child(divider().mt_1()).child(
                    h_flex().px_2p5().pt_1p5().pb_0p5().justify_end().child(
                        jig_commands::surface::hint(
                            "↑↓ navigate · ↩ open · ⌥C ⌥W ⌥X options · esc close",
                            cx,
                        ),
                    ),
                )
            });
        jig_commands::motion::pop_in(panel, "jig-find-in-files")
    }
}
