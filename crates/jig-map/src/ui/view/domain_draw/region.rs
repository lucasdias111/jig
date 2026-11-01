//! Drawing a domain's region and the labels on its links.

use super::*;

impl MapView {
    /// A domain, Shared or Unsorted, as a region, and its name above it.
    pub(in crate::ui::view) fn render_region(
        &self,
        region: &RegionBox,
        view: View,
        lit: &Lit,
        cx: &Context<Self>,
    ) -> [AnyElement; 2] {
        let theme = cx.theme();
        let dark = theme.mode.is_dark();
        let agent = agent_accent(cx);
        let quiet = !matches!(region.kind, RegionKind::Domain(_));
        let colour = match region.kind {
            RegionKind::Domain(ix) if !region.draft => domain::colour(ix, dark),
            _ => theme.muted_foreground,
        };
        let at = view.point((region.rect.x, region.rect.y));
        let (width, height) = (
            view.scaled(region.rect.width),
            view.scaled(region.rect.height),
        );
        let selected = lit
            .selected
            .iter()
            .any(|key| key.strip_prefix("site:").unwrap_or(key) == region.key);

        let target = self.drop_target() == Some(region.ix);
        let header = if region.open > 0. {
            view.scaled(region.header).min(height)
        } else {
            height
        };
        let dashed = region.draft || region.kind == RegionKind::Unsorted;
        // Picked (selected, or holding what's selected): lit in its own
        // colour, outline and fill.
        let lit_up = selected || region.lit;
        let border = if target {
            theme.primary
        } else if region.proposed {
            agent
        } else if lit_up {
            colour
        } else {
            colour.opacity(if quiet { 0.35 } else { 0.55 })
        };
        let editing = self.domains.editing.as_ref().and_then(|editing| {
            (RegionKind::Domain(editing.domain) == region.kind).then_some(editing.field)
        });
        let name_colour = if quiet || region.draft {
            theme.foreground.opacity(0.75)
        } else {
            blend(colour, theme.foreground, 0.25)
        };
        let ix = region.ix;
        let offers = (!region.offers.is_empty()).then(|| {
            h_flex()
                .flex_wrap()
                .gap(px(4.))
                .children(region.offers.iter().map(|offer| {
                    div()
                        .px(px(6.))
                        .py(px(1.))
                        .rounded(px(4.))
                        .bg(colour.opacity(0.14))
                        .text_size(px(10.5))
                        .text_color(theme.foreground.opacity(0.8))
                        .child(offer.clone())
                }))
        });
        let about = if region.draft {
            region.why.clone()
        } else {
            region.purpose.clone()
        };
        let body = div()
            .id(("map-region", ix))
            .debug_selector(move || format!("map-region-{ix}"))
            .absolute()
            .left(at.x)
            .top(at.y)
            .w(width)
            .h(height)
            .rounded(px(14.))
            .border_1()
            .when(lit_up || target || region.proposed, |this| this.border_2())
            .when(selected, |this| this.shadow_lg())
            .when(dashed, |this| this.border_dashed())
            .border_color(border)
            .bg(blend(
                theme::canvas(cx),
                colour,
                match (quiet || region.draft, lit_up) {
                    (true, false) => 0.04,
                    (true, true) => 0.09,
                    (false, false) => 0.1,
                    (false, true) => 0.17,
                },
            ))
            .when(region.proposed, |this| this.shadow_md())
            .opacity(region.alpha)
            .overflow_hidden()
            .child(
                v_flex()
                    .h(header)
                    .overflow_hidden()
                    .px(px(14.))
                    .py(px(10.))
                    .gap(px(6.))
                    .child(
                        h_flex()
                            .gap_2()
                            .items_baseline()
                            .min_w_0()
                            .child(
                                div()
                                    .min_w_0()
                                    .text_size(px(10.5))
                                    .text_color(theme.muted_foreground)
                                    .child(region.note.clone()),
                            )
                            .when(region.proposed, |this| {
                                this.child(
                                    h_flex()
                                        .flex_none()
                                        .gap_1()
                                        .px_1()
                                        .rounded(px(4.))
                                        .bg(agent.opacity(0.14))
                                        .text_size(px(10.))
                                        .text_color(agent)
                                        .child(agent_icon().size(px(10.)).text_color(agent))
                                        .child("Proposed"),
                                )
                            }),
                    )
                    .when(!about.is_empty(), |this| {
                        this.child(
                            div()
                                .text_size(px(if quiet { 11.5 } else { 12.5 }))
                                .line_height(px(17.))
                                .text_color(theme.foreground.opacity(if quiet {
                                    0.6
                                } else {
                                    0.85
                                }))
                                .when(editing == Some(Field::Purpose), |this| this.opacity(0.))
                                .child(about),
                        )
                    })
                    .children(offers),
            )
            .into_any_element();
        // The name above the region, in its colour, as long as it is.
        let name = name_label(
            &region.name,
            quiet,
            name_room(f32::from(width), view.camera.zoom),
        );
        let label = v_flex()
            .id(("map-region-name", ix))
            .debug_selector(move || format!("map-region-name-{ix}"))
            .absolute()
            .left(at.x + px(4.))
            .top(at.y - px(name.height))
            .h(px(name.height - 4.))
            .justify_end()
            .whitespace_nowrap()
            .text_size(px(name.size))
            .line_height(px(name.size * 1.25))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(name_colour)
            .opacity(region.alpha)
            .when(editing == Some(Field::Name), |this| this.opacity(0.))
            .children(name.lines.into_iter().map(|line| div().child(line)))
            .into_any_element();
        [body, label]
    }

    /// What flows along a link, each way, in a label at `spot` (its top
    /// left, out of every region's way), `size` big.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::ui::view) fn render_flow_label(
        &self,
        ix: usize,
        arrow: &Arrow,
        spot: (f32, f32),
        size: (f32, f32),
        from: Point<Pixels>,
        to: Point<Pixels>,
        cx: &Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let dark = theme.mode.is_dark();
        let (dx, dy) = (f32::from(to.x - from.x), f32::from(to.y - from.y));
        let line_colour = match arrow.tint {
            Some(tint) => domain::colour(tint, dark),
            None => theme.foreground,
        };
        // The selected domain's: headed by the domain at the other end,
        // its words read from the selected side.
        let title = arrow.title.clone().map(|title| {
            div()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(match arrow.title_tint {
                    Some(tint) => domain::colour(tint, dark),
                    None => theme.foreground,
                })
                .child(title)
        });
        let relative = arrow.title.is_some();
        v_flex()
            .id(("map-flow", ix))
            .debug_selector(move || format!("map-flow-{ix}"))
            .absolute()
            .left(px(spot.0))
            .top(px(spot.1))
            .w(px(size.0))
            .px(px(6.))
            .py(px(2.))
            .rounded(px(6.))
            .when(!arrow.faint, |this| {
                this.bg(theme::canvas(cx))
                    .border_1()
                    .border_color(if arrow.picked {
                        line_colour.opacity(0.7)
                    } else {
                        theme.foreground.opacity(0.18)
                    })
            })
            .when(arrow.faint, |this| {
                this.border_1().border_color(transparent_black())
            })
            .text_size(px(10.5))
            .line_height(px(14.))
            .text_color(theme.foreground.opacity(0.75))
            .opacity(arrow.alpha * if arrow.faint { 0.3 } else { 1. })
            .children(title)
            .children(arrow.labels.iter().map(move |(forward, label)| {
                let glyph = match (relative, forward) {
                    (true, true) => "→",
                    (true, false) => "←",
                    (false, _) => glyph(*forward, dx, dy),
                };
                div().child(format!("{glyph} {label}"))
            }))
            .into_any_element()
    }

    /// The region a dragged file or domain would be dropped on.
    pub(super) fn drop_target(&self) -> Option<usize> {
        let at = match &self.drag {
            Some(Drag::Member { at, .. }) | Some(Drag::Region { at, .. }) => *at,
            _ => return None,
        };
        (self.level == Level::Domain)
            .then(|| self.region_at(at))
            .flatten()
    }
}
