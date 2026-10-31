//! What's drawn over the canvas: the marquee, the hover peek and the hint.

use super::*;

impl MapView {
    pub(super) fn render_marquee(&self, cx: &App) -> Option<AnyElement> {
        let Some(super::super::input::Drag::Marquee { from, to, .. }) = &self.drag else {
            return None;
        };
        let theme = cx.theme();
        let origin = self.bounds.get().origin;
        let (a, b) = (*from - origin, *to - origin);
        Some(
            div()
                .absolute()
                .left(a.x.min(b.x))
                .top(a.y.min(b.y))
                .w((a.x - b.x).abs())
                .h((a.y - b.y).abs())
                .border_1()
                .border_color(theme.foreground.opacity(0.5))
                .bg(theme.foreground.opacity(0.06))
                .into_any_element(),
        )
    }

    /// What the pointer rests on: a function's code at Plan, the calls
    /// behind an arrow at Site.
    pub(super) fn render_hover(&self, cx: &App) -> Option<AnyElement> {
        let theme = cx.theme();
        let (at, lines, mono) = match self.hover.as_ref()? {
            Hover::Card(key, since) if since.elapsed().as_millis() >= input::PEEK_MS => {
                let lines = self.peek(key)?;
                let ix = self.plan.index_of(key)?;
                let rect = self.placed.nodes[ix];
                (self.to_canvas((rect.x, rect.bottom() + 6.)), lines, true)
            }
            Hover::Arrow(ix, position) if self.level == Level::Domain => {
                let arrow = self.drawn.borrow().arrows.get(*ix)?.clone();
                let name = |key: &str| key.strip_prefix("domain:").unwrap_or(key).to_string();
                let (a, b) = (name(&arrow.keys.0), name(&arrow.keys.1));
                let lines = arrow
                    .labels
                    .iter()
                    .map(|(forward, label)| {
                        if *forward {
                            format!("{a} → {b}: {label}")
                        } else {
                            format!("{b} → {a}: {label}")
                        }
                    })
                    .collect();
                (
                    *position - self.bounds.get().origin + point(px(12.), px(12.)),
                    lines,
                    false,
                )
            }
            Hover::Arrow(ix, position) => {
                let arrow = self.drawn.borrow().arrows.get(*ix)?.clone();
                let calls = arrow.calls?;
                let mut lines = vec![format!(
                    "{} call{}",
                    calls.count,
                    if calls.count == 1 { "" } else { "s" }
                )];
                lines.extend(calls.examples.iter().cloned());
                if calls.count > calls.examples.len() {
                    lines.push("…".into());
                }
                (
                    *position - self.bounds.get().origin + point(px(12.), px(12.)),
                    lines,
                    false,
                )
            }
            _ => return None,
        };
        Some(
            v_flex()
                .id("map-hover")
                .debug_selector(|| "map-hover".into())
                .absolute()
                .left(at.x)
                .top(at.y)
                .max_w(px(520.))
                .p_2()
                .rounded(px(6.))
                .border_1()
                .border_color(theme.border)
                .bg(theme.popover)
                .shadow_lg()
                .text_size(px(11.))
                .when(mono, |this| {
                    this.font_family(theme.mono_font_family.clone())
                })
                .text_color(theme.popover_foreground)
                .children(
                    lines
                        .into_iter()
                        .map(|line| div().whitespace_nowrap().truncate().child(line)),
                )
                .into_any_element(),
        )
    }

    pub(super) fn render_hint(&self, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let notice = self
            .notice
            .as_ref()
            .filter(|(_, at)| at.elapsed().as_secs() < 4)
            .map(|(text, _)| text.clone());
        let drawing = match self.tool() {
            super::super::sketch::Tool::Pointer => None,
            super::super::sketch::Tool::Arrow => {
                Some("Drag from a box to another, or off a box to a new one · Esc for the pointer")
            }
            super::super::sketch::Tool::Note => Some(
                "Click a box to note on it, or anywhere for a loose note · Esc for the pointer",
            ),
            _ => Some("Click or drag to draw · Esc for the pointer"),
        };
        let text = notice.or(drawing.map(String::from)).unwrap_or_else(|| {
            match self.level {
                Level::Domain if self.drafts() => {
                    "Jig's own grouping, from folders and calls · zoom into a group to open it onto its code · ⌘↓ or Enter enters"
                }
                Level::Domain => {
                    "Zoom into a domain to open it onto its code · drop a domain onto another to merge · double-click a name to rename · ⌘Z undoes"
                }
                Level::Site => {
                    "Zoom in: each domain opens onto its components, folders and files, a file into its Plan · ⌘↓, Enter or double-click enters · ⌘↑ or zoom out to Domain"
                }
                Level::Plan(_) => {
                    "Zoom into a function for its Plan, into this one for its code · double-click or Enter enters · ◀ ▶ open a hop · ⌘↑ out to Site · Esc back to the code"
                }
            }
            .to_string()
        });
        div()
            .absolute()
            .bottom(px(10.))
            .left(px(14.))
            .right(px(14.))
            .truncate()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(text)
    }
}
