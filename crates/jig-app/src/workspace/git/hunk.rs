//! The popup with a change's old lines, opened by clicking its bar in the
//! gutter. An entity of its own: the workspace opens it on a change and
//! puts the lines back when it asks.

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use jig_git::Hunk;

/// Lines of a change the popup shows on each side before cutting it short.
const POPUP_LINES: usize = 16;
const POPUP_WIDTH: f32 = 520.;

#[derive(Default)]
pub(in crate::workspace) struct HunkPopup {
    open: Option<OpenHunk>,
}

struct OpenHunk {
    tab: EntityId,
    hunk: Hunk,
    /// The change's lines as they now are. Any edit to the tab closes the
    /// popup, so they stay current while it shows.
    new_text: String,
    anchor: Point<Pixels>,
}

/// Revert was clicked: put change `hunk` in tab `tab` back as it was
/// committed. The popup has closed.
pub(in crate::workspace) struct RevertHunk {
    pub(in crate::workspace) tab: EntityId,
    pub(in crate::workspace) hunk: Hunk,
}

impl EventEmitter<RevertHunk> for HunkPopup {}

impl HunkPopup {
    #[cfg(test)]
    pub(in crate::workspace) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Whether it shows `hunk` of tab `tab`.
    pub(in crate::workspace) fn shows(&self, tab: EntityId, hunk: &Hunk) -> bool {
        self.open
            .as_ref()
            .is_some_and(|open| open.tab == tab && open.hunk == *hunk)
    }

    /// The tab whose change it shows, while it's open.
    pub(in crate::workspace) fn tab(&self) -> Option<EntityId> {
        self.open.as_ref().map(|open| open.tab)
    }

    /// Show `hunk` of tab `tab`, whose lines now read `new_text`, at
    /// `anchor`.
    pub(in crate::workspace) fn open(
        &mut self,
        tab: EntityId,
        hunk: Hunk,
        new_text: String,
        anchor: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.open = Some(OpenHunk {
            tab,
            hunk,
            new_text,
            anchor,
        });
        cx.notify();
    }

    /// Close it. Returns whether it was open.
    pub(in crate::workspace) fn close(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.open.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    fn revert(&mut self, cx: &mut Context<Self>) {
        if let Some(open) = self.open.take() {
            cx.emit(RevertHunk {
                tab: open.tab,
                hunk: open.hunk,
            });
            cx.notify();
        }
    }
}

impl Render for HunkPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(open) = &self.open else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let hunk = &open.hunk;
        let title = if hunk.is_deletion() {
            plural(
                hunk.old_text.lines().count(),
                "line removed",
                "lines removed",
            )
        } else if hunk.is_addition() {
            plural(hunk.lines.len(), "line added", "lines added")
        } else {
            plural(hunk.lines.len(), "line changed", "lines changed")
        };
        let lines = |text: &str, sign: &'static str, color: Hsla| {
            let all: Vec<&str> = text.lines().collect();
            let more = all.len().saturating_sub(POPUP_LINES);
            v_flex()
                .children(all.into_iter().take(POPUP_LINES).map(move |line| {
                    h_flex()
                        .px_2()
                        .bg(color.opacity(0.12))
                        .child(div().flex_none().w(px(14.)).text_color(color).child(sign))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .whitespace_nowrap()
                                .child(line.replace('\t', "    ")),
                        )
                }))
                .when(more > 0, |this| {
                    this.child(
                        div()
                            .px_2()
                            .text_color(theme.muted_foreground)
                            .child(format!("… {more} more")),
                    )
                })
        };
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.into())
                .px_2()
                .h(px(22.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .text_size(px(11.5))
                .text_color(theme.foreground.opacity(0.85))
                .hover(|s| s.bg(theme.foreground.opacity(0.08)))
                .child(label)
        };
        let panel = jig_commands::surface::panel(cx)
            .id("git-hunk")
            .w(px(POPUP_WIDTH))
            .max_w(px(POPUP_WIDTH))
            .p_1()
            .flex()
            .flex_col()
            .gap_1()
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                this.close(cx);
            }))
            .child(
                h_flex()
                    .pl_2()
                    .gap_1()
                    .child(
                        div()
                            .flex_1()
                            .text_size(px(11.5))
                            .text_color(theme.muted_foreground)
                            .child(format!("{title} since the last commit")),
                    )
                    .child(
                        button("git-revert", "Revert")
                            .on_click(cx.listener(|this, _, _, cx| this.revert(cx))),
                    )
                    .child(button("git-hunk-close", "Close").on_click(cx.listener(
                        |this, _, _, cx| {
                            this.close(cx);
                        },
                    ))),
            )
            .child(
                v_flex()
                    .rounded(px(6.))
                    .overflow_hidden()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(px(12.))
                    .child(lines(&hunk.old_text, "−", theme.red))
                    .child(lines(&open.new_text, "+", theme.green)),
            );
        deferred(
            anchored()
                .position(open.anchor)
                .snap_to_window_with_margin(px(8.))
                .child(panel),
        )
        .into_any_element()
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}
