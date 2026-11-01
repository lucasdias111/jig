//! The panel showing a problem under the pointer, with its fixes.

use super::*;

/// A problem's icon, message and origin, as its panel and the hover show
/// it.
pub(in crate::workspace) fn problem_row(entry: &DiagnosticEntry, cx: &App) -> Div {
    let theme = cx.theme();
    let color = severity_color(entry.severity, cx);
    let icon = match entry.severity {
        DiagnosticSeverity::Error => ERROR_ICON,
        DiagnosticSeverity::Warning => WARNING_ICON,
        DiagnosticSeverity::Info | DiagnosticSeverity::Hint => INFO_ICON,
    };
    let origin: Vec<SharedString> = [entry.source.clone(), entry.code.clone()]
        .into_iter()
        .flatten()
        .collect();
    div()
        .flex()
        .items_start()
        .gap_2()
        .child(
            Icon::default()
                .data(icon)
                .size(px(13.))
                .flex_none()
                .mt(px(2.))
                .text_color(color),
        )
        .child(
            v_flex()
                .min_w_0()
                .gap_0p5()
                .child(entry.message.clone())
                .when(!origin.is_empty(), |this| {
                    this.child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.muted_foreground)
                            .child(origin.join(" ")),
                    )
                }),
        )
}

/// The message of the problem under the pointer, or the one the keys went
/// to, in a small panel below it, with ways to fix it. A view of its own,
/// so hovering repaints only this.
pub(in crate::workspace) struct ProblemHover {
    pub(super) editor: Entity<EditorState>,
    /// Shown by the keys, while the cursor stays where they put it.
    pub(super) pinned: Option<(usize, Rc<DiagnosticEntry>)>,
    /// The problem the pointer was last on. It stays a moment after the
    /// pointer leaves, and while the pointer is over the panel, so the
    /// fixes can be reached.
    pub(super) last: Option<Rc<DiagnosticEntry>>,
    /// The pointer is over the panel.
    pub(super) held: bool,
    /// The pointer has left; `last` goes when this finishes.
    pub(super) leaving: Option<Task<()>>,
    /// The hover panel shows the problem, above what the server says about
    /// the symbol there.
    pub(super) under_hover: bool,
    pub(super) fixes: Entity<PanelFixes>,
    pub(super) _observe: Subscription,
}

impl ProblemHover {
    pub(super) fn new(
        editor: Entity<EditorState>,
        workspace: WeakEntity<Workspace>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe = cx.observe(&editor, |this, _, cx| this.editor_changed(cx));
        let fixes = cx.new(|_| PanelFixes::new(workspace, editor.entity_id()));
        Self {
            editor,
            pinned: None,
            last: None,
            held: false,
            leaving: None,
            under_hover: false,
            fixes,
            _observe: observe,
        }
    }

    /// Follow the pointer onto problems, and let the last one linger as it
    /// leaves.
    fn editor_changed(&mut self, cx: &mut Context<Self>) {
        if !self.held {
            match self.editor.read(cx).hovered_diagnostic() {
                Some(entry) => {
                    self.last = Some(entry);
                    self.leaving = None;
                }
                None => self.leave(cx),
            }
        }
        cx.notify();
    }

    /// The pointer left the problem or its panel: put it away after a
    /// moment, unless the pointer comes back.
    fn leave(&mut self, cx: &mut Context<Self>) {
        if self.last.is_none() || self.leaving.is_some() {
            return;
        }
        self.leaving = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(LINGER).await;
            this.update(cx, |this, cx| {
                this.leaving = None;
                if !this.held && this.editor.read(cx).hovered_diagnostic().is_none() {
                    this.last = None;
                    cx.notify();
                }
            })
            .ok();
        }));
    }

    fn set_held(&mut self, held: bool, cx: &mut Context<Self>) {
        if self.held == held {
            return;
        }
        self.held = held;
        if !held {
            self.editor_changed(cx);
        }
    }

    pub(super) fn pin(&mut self, entry: DiagnosticEntry, cx: &mut Context<Self>) {
        self.pinned = Some((entry.range.start, Rc::new(entry)));
        cx.notify();
    }

    pub(in crate::workspace) fn unpin(&mut self, cx: &mut Context<Self>) -> bool {
        let pinned = self.pinned.take().is_some();
        if pinned {
            cx.notify();
        }
        pinned
    }

    /// Put the panel away: a fix was taken.
    pub(in crate::workspace) fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.pinned = None;
        self.last = None;
        self.held = false;
        self.leaving = None;
        self.editor
            .update(cx, |editor, cx| editor.clear_diagnostic_popover(cx));
        cx.notify();
    }

    pub(in crate::workspace) fn set_under_hover(&mut self, under: bool, cx: &mut Context<Self>) {
        if self.under_hover != under {
            self.under_hover = under;
            cx.notify();
        }
    }

    /// Whether the pointer is over the panel, which keeps the hover of
    /// whatever is under it away.
    pub(in crate::workspace) fn is_held(&self) -> bool {
        self.held
    }

    /// The problem the pointer is on, or was on a moment ago.
    pub(in crate::workspace) fn pointed_at(&self, cx: &App) -> Option<Rc<DiagnosticEntry>> {
        if self.held {
            return self.last.clone();
        }
        self.editor
            .read(cx)
            .hovered_diagnostic()
            .or_else(|| self.last.clone())
    }

    /// What's shown: the pointer's problem, else the pinned one.
    pub(super) fn shown(&mut self, cx: &App) -> Option<Rc<DiagnosticEntry>> {
        let state = self.editor.read(cx);
        if let Some((at, _)) = &self.pinned
            && (state.cursor() != *at || !state.selected_range().is_empty())
        {
            self.pinned = None;
        }
        self.pointed_at(cx)
            .or_else(|| self.pinned.as_ref().map(|(_, entry)| entry.clone()))
    }

    #[cfg(test)]
    pub(in crate::workspace) fn message(&mut self, cx: &App) -> Option<String> {
        self.shown(cx).map(|entry| entry.message.to_string())
    }

    #[cfg(test)]
    pub(in crate::workspace) fn fix_titles(&self, cx: &App) -> Vec<String> {
        self.fixes.read(cx).titles()
    }
}

impl Render for ProblemHover {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(entry) = self.shown(cx) else {
            return Empty.into_any_element();
        };
        let state = self.editor.read(cx);
        // The hover shows it, above what the server says about the symbol.
        if !self.held && (self.under_hover || state.hovered_symbol().is_some()) {
            return Empty.into_any_element();
        }
        let start = entry.range.start;
        // Below the line it starts on, or failing that (scrolled off), the
        // one it ends on.
        let Some(line) = state
            .range_to_bounds(&(start..start))
            .or_else(|| state.range_to_bounds(&(entry.range.end..entry.range.end)))
        else {
            return Empty.into_any_element();
        };
        self.fixes
            .update(cx, |fixes, cx| fixes.offer_for(&entry, cx));
        let panel = jig_commands::surface::panel(cx)
            .id("problem")
            .debug_selector(|| "problem".into())
            .max_w(px(PANEL_MAX_WIDTH))
            .px_2p5()
            .py_1p5()
            .text_size(px(12.5))
            .on_hover(cx.listener(|this, held: &bool, _, cx| this.set_held(*held, cx)))
            .child(
                v_flex()
                    .gap_1p5()
                    .child(problem_row(&entry, cx))
                    .child(self.fixes.clone()),
            );
        deferred(
            anchored()
                .position(point(line.left() - px(10.), line.bottom() + px(4.)))
                .snap_to_window_with_margin(px(8.))
                .child(panel),
        )
        .into_any_element()
    }
}
