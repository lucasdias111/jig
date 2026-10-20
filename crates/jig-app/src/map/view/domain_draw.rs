//! Drawing the Domain level: each domain a big region in its own colour,
//! its name above it in that colour, never cut short, and inside at the
//! top how much it holds, its full purpose and what it offers people;
//! Shared and
//! Unsorted greyer, apart. Jig's drafts are dashed and colourless under a
//! banner asking to index them; a proposal is outlined in the agent's
//! colour with Keep and Discard. Each link carries its flows as a label.

use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_commands::surface::{agent_accent, agent_icon};

use super::super::domains::actions::Action;
use super::super::domains::level::RegionKind;
use super::domain::{self, Field};
use super::draw::{Lit, View, blend, theme};
use super::input::Drag;
use super::scene::{Arrow, RegionBox};
use super::{Level, MapView};

/// A region's name, above it: at most this big on screen.
const NAME_SIZE: f32 = 16.;
const QUIET_NAME_SIZE: f32 = 13.;
/// It shrinks to fit the room above its region down to this, then wraps.
const SMALLEST_NAME: f32 = 11.;

/// A region's name as drawn above it, on screen: its size, its lines
/// (never cut short) and how much room they take.
#[derive(Clone, Debug, PartialEq)]
pub struct NameLabel {
    pub size: f32,
    pub lines: Vec<String>,
    pub width: f32,
    pub height: f32,
}

/// Region `name` laid out to fit `room` points across: at its size if it
/// fits, else smaller, else on more lines.
pub fn name_label(name: &str, quiet: bool, room: f32) -> NameLabel {
    let most = if quiet { QUIET_NAME_SIZE } else { NAME_SIZE };
    let width_at = |text: &str, size: f32| text.chars().count() as f32 * size * 0.6;
    let size = (most * room / width_at(name, most).max(1.)).clamp(SMALLEST_NAME, most);
    let mut lines: Vec<String> = Vec::new();
    for word in name.split_whitespace() {
        match lines.last_mut() {
            Some(line) if width_at(&format!("{line} {word}"), size) <= room => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_string()),
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    let width = lines
        .iter()
        .map(|line| width_at(line, size))
        .fold(0., f32::max)
        + 8.;
    let height = lines.len() as f32 * (size * 1.25) + 6.;
    NameLabel {
        size,
        lines,
        width,
        height,
    }
}

/// The room a region `width` points wide on screen has for its name: its
/// width and most of the gap beside it, at `zoom`.
pub fn name_room(width: f32, zoom: f32) -> f32 {
    width + super::super::site::REGION_GAP * zoom * 0.8 - 8.
}

impl MapView {
    /// The header's actions at Domain: Index Domains, Update Domains when
    /// files are unsorted, Copy as Markdown.
    pub(super) fn render_domain_buttons(&self, cx: &Context<Self>) -> AnyElement {
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
    pub(super) fn render_export_button(&self, cx: &Context<Self>) -> AnyElement {
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

    /// A domain, Shared or Unsorted, as a region, and its name above it.
    pub(super) fn render_region(
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
        let selected = lit.selected.contains(&region.key);
        let target = self.drop_target() == Some(region.ix);
        let header = if region.open > 0. {
            view.scaled(region.header).min(height)
        } else {
            height
        };
        let dashed = region.draft || region.kind == RegionKind::Unsorted;
        let border = if target || selected {
            theme.primary
        } else if region.proposed {
            agent
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
            .when(selected || target || region.proposed, |this| {
                this.border_2()
            })
            .when(dashed, |this| this.border_dashed())
            .border_color(border)
            .bg(blend(
                theme::canvas(cx),
                colour,
                if quiet || region.draft { 0.04 } else { 0.1 },
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
    pub(super) fn render_flow_label(
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
        let (dx, dy) = (f32::from(to.x - from.x), f32::from(to.y - from.y));
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
            .bg(theme::canvas(cx))
            .border_1()
            .border_color(theme.foreground.opacity(0.18))
            .text_size(px(10.5))
            .line_height(px(14.))
            .text_color(theme.foreground.opacity(0.75))
            .opacity(arrow.alpha)
            .children(arrow.labels.iter().map(|(forward, label)| {
                div().child(format!("{} {label}", glyph(*forward, dx, dy)))
            }))
            .into_any_element()
    }

    /// The region a dragged file or domain would be dropped on.
    fn drop_target(&self) -> Option<usize> {
        let at = match &self.drag {
            Some(Drag::Member { at, .. }) | Some(Drag::Region { at, .. }) => *at,
            _ => return None,
        };
        (self.level == Level::Domain)
            .then(|| self.region_at(at))
            .flatten()
    }

    /// Over the canvas at Domain: the banner, the selected domain's
    /// actions, a name or purpose being edited, and what's being dragged.
    pub(super) fn render_domain_overlay(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let mut out = Vec::new();
        out.extend(self.render_banner(cx));
        out.extend(self.render_region_actions(cx));
        out.extend(self.render_edit_field(cx));
        out.extend(self.render_carried(cx));
        out
    }

    fn render_banner(&self, cx: &Context<Self>) -> Option<AnyElement> {
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
                .top(px(12.))
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
    fn render_region_actions(&self, cx: &Context<Self>) -> Option<AnyElement> {
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
    fn render_edit_field(&self, cx: &Context<Self>) -> Option<AnyElement> {
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
    fn render_carried(&self, cx: &Context<Self>) -> Option<AnyElement> {
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

/// The arrow a flow going forward (from where the link leaves) or back
/// reads with, for a link running `dx`, `dy`.
pub(super) fn glyph(forward: bool, dx: f32, dy: f32) -> &'static str {
    let (dx, dy) = if forward { (dx, dy) } else { (-dx, -dy) };
    if dx.abs() >= dy.abs() {
        if dx >= 0. { "→" } else { "←" }
    } else if dy >= 0. {
        "↓"
    } else {
        "↑"
    }
}
