//! Domain's buttons, banner, edit field and what's being carried.

use super::*;

impl MapView {
    /// The header's actions at Domain: Index Domains, Update Domains when
    /// files are unsorted, Copy as Markdown.
    pub(in crate::ui::view) fn render_domain_buttons(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let agent = agent_accent(cx);
        let unsorted = self
            .domain_level()
            .map(|level| level.unsorted().len())
            .unwrap_or(0);
        let busy = self.domains.working.is_some();
        let index_tip = if self.domains.saved.is_some() {
            "Ask the agent to index the domains again, from scratch"
        } else {
            "Ask the agent to name and describe the project's domains"
        };
        h_flex()
            .gap_1()
            .child(
                h_flex()
                    .id("map-index-domains")
                    .debug_selector(|| "map-index-domains".into())
                    .h(px(22.))
                    .px_2()
                    .gap_1()
                    .rounded(px(5.))
                    .text_xs()
                    .text_color(agent)
                    .when(!busy, |this| {
                        this.hover(|style| style.bg(agent.opacity(0.12))).on_click(
                            cx.listener(|this, _, _, cx| this.start_action(Action::Index, cx)),
                        )
                    })
                    .when(busy, |this| this.opacity(0.5))
                    .child(agent_icon().size(px(12.)).text_color(agent))
                    .child("Index Domains")
                    .tooltip(move |window, cx| Tooltip::new(index_tip).build(window, cx)),
            )
            .when(self.domains.saved.is_some() && unsorted > 0, |this| {
                this.child(
                    h_flex()
                        .id("map-update-domains")
                        .debug_selector(|| "map-update-domains".into())
                        .h(px(22.))
                        .px_2()
                        .gap_1()
                        .rounded(px(5.))
                        .text_xs()
                        .text_color(agent)
                        .when(!busy, |this| {
                            this.hover(|style| style.bg(agent.opacity(0.12)))
                                .on_click(cx.listener(|this, _, _, cx| this.update_domains(cx)))
                        })
                        .when(busy, |this| this.opacity(0.5))
                        .child(agent_icon().size(px(12.)).text_color(agent))
                        .child(format!("Update Domains ({unsorted})"))
                        .tooltip(|window, cx| {
                            Tooltip::new("Ask the agent to place the unsorted files only")
                                .build(window, cx)
                        }),
                )
            })
            .child(
                h_flex()
                    .id("map-copy-markdown")
                    .debug_selector(|| "map-copy-markdown".into())
                    .h(px(22.))
                    .px_2()
                    .rounded(px(5.))
                    .text_xs()
                    .text_color(theme.foreground.opacity(0.85))
                    .hover(|style| style.bg(theme.foreground.opacity(0.08)))
                    .child("Copy as Markdown")
                    .tooltip(|window, cx| {
                        Tooltip::new("Each domain's name, purpose and offers, and the flows")
                            .build(window, cx)
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.copy_markdown(cx))),
            )
            .into_any_element()
    }

    /// Export Image: Copy Image or Save Image… of the level showing.
    pub(in crate::ui::view) fn render_export_button(&self, cx: &Context<Self>) -> AnyElement {
        let view = cx.weak_entity();
        Button::new("map-export")
            .label("Export Image")
            .ghost()
            .xsmall()
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let (copy, save) = (view.clone(), view.clone());
                menu.item(PopupMenuItem::new("Copy Image").on_click(move |_, _, cx| {
                    copy.update(cx, |this, cx| this.copy_image(cx)).ok();
                }))
                .item(PopupMenuItem::new("Save Image…").on_click(move |_, _, cx| {
                    save.update(cx, |this, cx| this.save_image(cx)).ok();
                }))
            })
            .into_any_element()
    }

    pub(super) fn render_banner(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let text = self.domain_banner()?;
        let theme = cx.theme();
        let agent = agent_accent(cx);
        let proposal = self.domains.proposal.is_some() && self.domains.working.is_none();
        let drafts = self.drafts() && self.domains.working.is_none();
        let pill = |id: &'static str, label: &'static str, primary: bool| {
            h_flex()
                .id(id)
                .debug_selector(move || id.into())
                .flex_none()
                .h(px(22.))
                .px_2()
                .gap_1()
                .rounded(px(5.))
                .text_xs()
                .when(primary, |this| this.bg(agent).text_color(gpui_kit::white()))
                .when(!primary, |this| {
                    this.border_1()
                        .border_color(theme.border)
                        .text_color(theme.foreground)
                        .hover(|style| style.bg(theme.foreground.opacity(0.06)))
                })
                .child(label)
        };
        Some(
            div()
                .absolute()
                .top(px(super::super::ruler::RULER_ROOM))
                .left(px(0.))
                .right(px(0.))
                .flex()
                .justify_center()
                .child(
                    h_flex()
                        .id("map-domain-banner")
                        .debug_selector(|| "map-domain-banner".into())
                        .gap_2()
                        .px_3()
                        .py(px(6.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(agent.opacity(0.5))
                        .bg(blend(theme.background, agent, 0.08))
                        .shadow_md()
                        .text_sm()
                        .child(agent_icon().size(px(14.)).text_color(agent))
                        .child(div().text_color(theme.foreground).child(text))
                        .when(drafts, |this| {
                            this.child(pill("map-banner-index", "Index Domains", true).on_click(
                                cx.listener(|this, _, _, cx| this.start_action(Action::Index, cx)),
                            ))
                        })
                        .when(proposal, |this| {
                            this.child(
                                div().text_xs().text_color(theme.muted_foreground).child(
                                    "Edit it here, or reply in the conversation to redo parts",
                                ),
                            )
                            .child(
                                pill("map-keep", "Keep ⌘↩", true)
                                    .on_click(cx.listener(|this, _, _, cx| this.keep_proposal(cx))),
                            )
                            .child(
                                pill("map-discard", "Discard", false).on_click(
                                    cx.listener(|this, _, _, cx| this.discard_proposal(cx)),
                                ),
                            )
                        }),
                )
                .into_any_element(),
        )
    }

    /// Above the one domain selected: Rename, Purpose, and Split with the
    /// agent.
    pub(super) fn render_region_actions(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.level != Level::Domain
            || self.drafts()
            || self.drag.is_some()
            || self.domains.editing.is_some()
            || self.transition.is_some()
        {
            return None;
        }
        let [key] = self.selected.as_slice() else {
            return None;
        };
        let level = self.domain_level()?;
        let ix = self.region_by_key(key)?;
        let domain = self.domain_of_region(ix)?;
        let rect = level.regions[ix].rect;
        let theme = cx.theme();
        let at = self.to_canvas((rect.right(), rect.y));
        let can_split = self.domains.proposal.is_none() && self.domains.working.is_none();
        let action = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .px_2()
                .h(px(20.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .text_xs()
                .text_color(theme.foreground.opacity(0.85))
                .hover(|style| style.bg(theme.foreground.opacity(0.08)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(label)
        };
        Some(
            h_flex()
                .absolute()
                .left(at.x - px(250.))
                .top(at.y - px(28.))
                .w(px(250.))
                .justify_end()
                .child(
                    h_flex()
                        .gap(px(2.))
                        .p(px(2.))
                        .rounded(px(6.))
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.popover)
                        .shadow_sm()
                        .child(action("map-rename-domain", "Rename").on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.begin_edit(domain, Field::Name, window, cx)
                            },
                        )))
                        .child(
                            action("map-domain-purpose", "Purpose").on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.begin_edit(domain, Field::Purpose, window, cx)
                                },
                            )),
                        )
                        .when(can_split, |this| {
                            let agent = agent_accent(cx);
                            this.child(
                                action("map-split-domain", "Split…")
                                    .text_color(agent)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.split_domain(domain, cx)
                                    }))
                                    .tooltip(|window, cx| {
                                        Tooltip::new("Ask the agent to split this domain")
                                            .build(window, cx)
                                    }),
                            )
                        }),
                )
                .into_any_element(),
        )
    }

    /// The name or purpose being edited, over where it's drawn.
    pub(super) fn render_edit_field(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let editing = self.domains.editing.as_ref()?;
        let level = self.domain_level()?;
        let region = level
            .regions
            .iter()
            .find(|region| region.kind == RegionKind::Domain(editing.domain))?;
        let rect = region.rect;
        let at = self.to_canvas((rect.x, rect.y));
        let width = (rect.width * self.zoom - 24.).clamp(200., 560.);
        let top = match editing.field {
            Field::Name => px(-34.),
            Field::Purpose => px(30.),
        };
        let theme = cx.theme();
        Some(
            div()
                .absolute()
                .left(at.x + px(10.))
                .top(at.y + top)
                .w(px(width))
                .rounded(px(6.))
                .bg(theme.background)
                .shadow_md()
                .child(Input::new(&editing.input).small().cleanable(false))
                .into_any_element(),
        )
    }

    /// A file or domain being carried, and where it would go.
    pub(super) fn render_carried(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let (title, at, merge) = match &self.drag {
            Some(Drag::Member { title, at, .. }) => (title.clone(), *at, false),
            Some(Drag::Region { region, at }) => {
                let level = self.domain_level()?;
                (level.regions.get(*region)?.name.clone(), *at, true)
            }
            _ => return None,
        };
        let theme = cx.theme();
        let to = if merge {
            self.region_at(at)
                .and_then(|ix| self.domain_of_region(ix).map(|_| ix))
                .and_then(|ix| self.domain_level().map(|l| l.regions[ix].name.clone()))
                .filter(|name| *name != title)
                .map(|name| format!("Merge into {name}"))
        } else {
            self.drop_home(at).map(|(_, name)| format!("→ {name}"))
        };
        let at = at - self.bounds.get().origin + point(px(12.), px(10.));
        Some(
            h_flex()
                .absolute()
                .left(at.x)
                .top(at.y)
                .gap_2()
                .px_2()
                .py_1()
                .rounded(px(6.))
                .border_1()
                .border_color(theme.primary)
                .bg(theme.popover)
                .shadow_lg()
                .text_xs()
                .child(div().font_weight(FontWeight::SEMIBOLD).child(title))
                .children(to.map(|to| div().text_color(theme.primary).child(to)))
                .into_any_element(),
        )
    }
}
