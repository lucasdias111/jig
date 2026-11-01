//! The Map's header: where you are, the level's actions and its switches.

use super::*;

impl MapView {
    /// Where you are: the project, the level, what it's about.
    fn crumbs(&self) -> Vec<String> {
        let project = self
            .root
            .file_name()
            .map_or("Project".into(), |name| name.to_string_lossy().into_owned());
        let mut crumbs = vec![project];
        match &self.level {
            Level::Site | Level::Domain => {}
            Level::Plan(Focus::Module(place)) => crumbs.push(place.trim_end_matches('/').into()),
            Level::Plan(Focus::Function(symbol)) => {
                crumbs.push(symbol.file.clone());
                crumbs.push(symbol.title());
            }
        }
        crumbs
    }

    pub(in crate::ui::view) fn render_header(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let crumbs = self.crumbs();
        let last = crumbs.len() - 1;
        let mut trail: Vec<AnyElement> = Vec::new();
        for (ix, label) in crumbs.into_iter().enumerate() {
            if ix > 0 {
                trail.push(
                    div()
                        .text_color(theme.muted_foreground)
                        .child("›")
                        .into_any_element(),
                );
            }
            trail.push(
                div()
                    .px_1()
                    .truncate()
                    .text_color(if ix == last {
                        theme.foreground
                    } else {
                        theme.muted_foreground
                    })
                    .child(label)
                    .into_any_element(),
            );
        }
        let button = |id: &'static str, label: &'static str, tip: &'static str| {
            h_flex()
                .id(id)
                .debug_selector(move || id.into())
                .flex_none()
                .h(px(22.))
                .px_2()
                .gap_1()
                .rounded(px(5.))
                .text_xs()
                .text_color(theme.foreground.opacity(0.85))
                .hover(|style| style.bg(theme.foreground.opacity(0.08)))
                .child(label)
                .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
        };
        let plan = matches!(self.level, Level::Plan(_));
        let tints = (self.has_tints() && self.level != Level::Domain).then(|| {
            h_flex()
                .id("map-tints")
                .gap_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(
                    Switch::new("map-tints-switch")
                        .small()
                        .checked(!self.domains.tints_off)
                        .on_click(cx.listener(|this, on: &bool, _, cx| this.set_tints(*on, cx))),
                )
                .child("Domains")
                .tooltip(|window, cx| {
                    Tooltip::new("Tint each box with its domain's colour").build(window, cx)
                })
        });
        let site = self.level == Level::Site;
        let tests = site.then(|| {
            h_flex()
                .id("map-tests")
                .gap_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(
                    Switch::new("map-tests-switch")
                        .small()
                        .checked(self.site_show.tests)
                        .on_click(cx.listener(|this, on: &bool, _, cx| {
                            this.set_site_show(Show { tests: *on }, cx)
                        })),
                )
                .child("Tests")
                .tooltip(|window, cx| {
                    Tooltip::new("Show the test files, and the calls they make").build(window, cx)
                })
        });
        let pinned = (!self.lens.pins.is_empty()).then(|| {
            h_flex()
                .id("map-pinned")
                .gap_1()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(
                    Switch::new("map-pinned-switch")
                        .small()
                        .checked(self.lens.pins_shown)
                        .on_click(
                            cx.listener(|this, on: &bool, _, cx| this.set_pins_shown(*on, cx)),
                        ),
                )
                .child("Pinned")
                .tooltip(|window, cx| {
                    Tooltip::new("Show the agent's drawings pinned to the Map").build(window, cx)
                })
        });
        let agent = jig_commands::surface::agent_accent(cx);
        let explain = self.project.is_some().then(|| {
            button(
                "map-explain",
                "Explain",
                "Ask the agent to explain this level",
            )
            .text_color(agent)
            .child(
                jig_commands::surface::agent_icon()
                    .size(px(11.))
                    .text_color(agent),
            )
            .on_click(cx.listener(|this, _, _, cx| this.explain_level(cx)))
        });
        let domain_buttons = (self.level == Level::Domain).then(|| self.render_domain_buttons(cx));
        let sketches = (!self.sketches.sketches.is_empty()).then(|| {
            button(
                "map-sketches",
                "Sketches",
                "Every sketch, archived ones too",
            )
            .when(self.sketches.list_open, |this| {
                this.bg(theme.foreground.opacity(0.1))
            })
            .on_click(cx.listener(|this, _, _, cx| {
                this.sketches.list_open = !this.sketches.list_open;
                cx.notify();
            }))
        });
        let export = self.render_export_button(cx);
        h_flex()
            .flex_none()
            .h(px(BAR_HEIGHT))
            .px_3()
            .gap_3()
            .justify_between()
            .border_b_1()
            .border_color(theme.title_bar_border)
            .child(
                h_flex()
                    .min_w_0()
                    .gap_1()
                    .text_sm()
                    .child(
                        Icon::new(IconName::Map)
                            .size(px(13.))
                            .flex_none()
                            .text_color(theme.muted_foreground),
                    )
                    .children(trail),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_2()
                    .children(explain)
                    .children(sketches)
                    .children(domain_buttons)
                    .children(pinned)
                    .children(tests)
                    .children(tints)
                    .child(export)
                    .when(plan, |this| {
                        this.child(
                            button("map-tidy", "Tidy", "Lay the boxes out afresh")
                                .on_click(cx.listener(|this, _, _, cx| this.tidy(cx))),
                        )
                        .child(
                            h_flex()
                                .id("map-live")
                                .gap_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(
                                    Switch::new("map-live-switch")
                                        .small()
                                        .checked(self.live)
                                        .on_click(cx.listener(|this, on: &bool, _, cx| {
                                            this.set_live(*on, cx)
                                        })),
                                )
                                .child("Live")
                                .tooltip(|window, cx| {
                                    Tooltip::new(
                                        "While debugging, functions glow as the program enters them",
                                    )
                                    .build(window, cx)
                                }),
                        )
                    }),
            )
    }
}
