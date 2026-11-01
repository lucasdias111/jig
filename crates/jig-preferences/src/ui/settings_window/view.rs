//! Drawing the Settings window: its sidebar, title bar, search results and
//! the page showing.

use super::*;

impl SettingsWindow {
    /// Show a failed change to the providers until the next one works.
    pub(super) fn report(&mut self, result: Result<(), String>, cx: &mut Context<Self>) {
        self.providers_error = result.err().map(SharedString::from);
        cx.notify();
    }

    fn render_sidebar(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let searching = !self.query(cx).is_empty();
        let item =
            |page: Page| {
                let selected = !searching && self.nav.page == page;
                let this = cx.entity().downgrade();
                let hover = theme.sidebar_accent.opacity(0.6);
                h_flex()
                    .id(SharedString::from(format!("page-{}", page.title())))
                    .h(px(28.))
                    .px_2()
                    .gap_2()
                    .rounded(px(6.))
                    .text_size(px(ui::TEXT))
                    .when(selected, |this| {
                        this.bg(theme.sidebar_accent)
                            .text_color(theme.sidebar_accent_foreground)
                    })
                    .when(!selected, |this| this.hover(move |style| style.bg(hover)))
                    .child(Icon::default().data(page.icon()).size(px(15.)).text_color(
                        if selected {
                            theme.sidebar_accent_foreground
                        } else {
                            theme.muted_foreground
                        },
                    ))
                    .child(page.title())
                    .on_click(move |_, window, cx| {
                        this.update(cx, |this, cx| this.go(Nav::page(page), window, cx))
                            .ok();
                    })
            };
        v_flex()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_shrink_0()
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .border_r_1()
            .border_color(ui::hairline(cx))
            .pt(px(TITLE_BAR_HEIGHT))
            .px_2p5()
            .gap_0p5()
            .child(
                div().pb_3().child(
                    Input::new(&self.search).small().cleanable(true).prefix(
                        Icon::new(IconName::Search)
                            .size(px(14.))
                            .text_color(theme.muted_foreground),
                    ),
                ),
            )
            .children(
                Page::ALL
                    .into_iter()
                    .filter(|page| !page.is_ai())
                    .map(&item),
            )
            .child(
                div()
                    .pt_4()
                    .pb_1()
                    .px_2()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.muted_foreground)
                    .child("AI"),
            )
            .children(Page::ALL.into_iter().filter(|page| page.is_ai()).map(item))
    }

    /// Every row matching the search, under the page and section it's in.
    fn search_results(&self, query: &str, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let mut found = Vec::new();
        // A row shown on two pages, like a debugger, is listed once.
        let mut seen = std::collections::HashSet::new();
        let languages = &settings::get(cx).languages;
        let shown = |nav: &Nav| match nav.detail {
            Some(Detail::Language(ix)) => languages.is_added(language(ix).name),
            _ => true,
        };
        for nav in Nav::everywhere().filter(shown) {
            let place = match nav.detail {
                Some(_) => format!("{} › {}", nav.page.title(), nav.title()),
                None => nav.page.title().to_string(),
            };
            for section in self.sections(nav, cx) {
                if let Some(mut section) = section.matching(query) {
                    section.rows.retain(|row| seen.insert(row.id.clone()));
                    if section.rows.is_empty() {
                        continue;
                    }
                    section.title = Some(match &section.title {
                        Some(title) => format!("{place} › {title}").into(),
                        None => place.clone().into(),
                    });
                    section.accessory = None;
                    found.push(section);
                }
            }
        }
        if found.is_empty() {
            return v_flex().pt_16().items_center().child(
                div()
                    .text_size(px(ui::TEXT))
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("No settings match “{query}”.")),
            );
        }
        ui::render_sections("search", &found, window, cx)
    }

    fn render_title(&self, cx: &Context<Self>) -> impl IntoElement {
        let searching = !self.query(cx).is_empty();
        let title: SharedString = if searching {
            "Search Results".into()
        } else {
            self.nav.title()
        };
        let back = (!searching && self.nav.detail.is_some()).then(|| {
            let this = cx.entity().downgrade();
            let page = self.nav.page;
            Button::new("back")
                .icon(IconName::ChevronLeft)
                .ghost()
                .small()
                .tooltip(format!("Back to {}", page.title()))
                .on_click(move |_, window, cx| {
                    this.update(cx, |this, cx| this.go(Nav::page(page), window, cx))
                        .ok();
                })
        });
        h_flex()
            .size_full()
            .pl(px(SIDEBAR_WIDTH + 16.))
            .gap_1()
            .items_center()
            .children(back)
            .child(
                div()
                    .text_size(px(15.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_color_pickers(window, cx);
        if self.nav.page == Page::AgentMode {
            self.look_up_agent_models(cx);
        }
        let query = self.query(cx);
        let content = if query.is_empty() {
            let sections = self.sections(self.nav, cx);
            ui::render_sections(&format!("{:?}", self.nav), &sections, window, cx)
        } else {
            self.search_results(&query, window, cx)
        };
        let theme = cx.theme();
        div()
            .size_full()
            .relative()
            .bg(theme.background)
            .text_color(theme.foreground)
            .child(
                h_flex().size_full().child(self.render_sidebar(cx)).child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(
                            div()
                                .h(px(TITLE_BAR_HEIGHT))
                                .flex_shrink_0()
                                .border_b_1()
                                .border_color(ui::hairline(cx)),
                        )
                        .child(
                            div()
                                .relative()
                                .flex_1()
                                .min_h_0()
                                .child(
                                    div()
                                        .id("settings-page")
                                        .size_full()
                                        .overflow_y_scroll()
                                        .track_scroll(&self.scroll)
                                        .child(
                                            h_flex().w_full().justify_center().child(
                                                div()
                                                    .w_full()
                                                    .max_w(px(CONTENT_WIDTH))
                                                    .px_6()
                                                    .pt_5()
                                                    .pb_10()
                                                    .child(content),
                                            ),
                                        ),
                                )
                                .vertical_scrollbar(&self.scroll),
                        ),
                ),
            )
            .child(
                // Over both columns, so the window drags from anywhere
                // along the top, as on the Mac.
                div().absolute().top_0().left_0().right_0().child(
                    TitleBar::new()
                        .h(px(TITLE_BAR_HEIGHT))
                        .pl_0()
                        .border_b_0()
                        .bg(transparent_black())
                        .child(self.render_title(cx)),
                ),
            )
    }
}
