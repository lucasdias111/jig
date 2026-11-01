//! The keys that settle an edit under review (Enter, Tab, Esc, ⌘Z) and when they go to the code instead.

use super::*;

impl Workspace {
    /// The agent's edit is under review in the tab on screen. One in
    /// another tab waits there.
    pub(in crate::workspace) fn previewing(&self) -> bool {
        self.agent_edit_here()
    }

    /// The agent's edit is under review in the tab on screen.
    pub(in crate::workspace) fn agent_edit_here(&self) -> bool {
        self.chat
            .as_ref()
            .is_some_and(|chat| chat.preview.is_some())
            && self.agent_edit_elsewhere().is_none()
    }

    /// Keep the edit on screen. It is already in the buffer as one undo
    /// step.
    pub(in crate::workspace) fn accept_preview(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.mode == super::super::mode::Mode::Code
            && self.agent_edit_here()
            && self.settle_agent_edit(true, Some(window), cx)
    }

    /// Drop the edit on screen by undoing it, which leaves no trace in the
    /// history beyond a redo step.
    fn reject_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.mode == super::super::mode::Mode::Code
            && self.agent_edit_here()
            && self.settle_agent_edit(false, Some(window), cx)
    }

    /// Enter accepts or allows; ⌘↩ allows the agent's request always.
    pub(in crate::workspace) fn on_accept_enter(
        &mut self,
        action: &Enter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if action.secondary
            && !self.keys_elsewhere(window, cx)
            && self.allow_agent_request_always(window, cx)
        {
            cx.stop_propagation();
            return;
        }
        self.accept_or_propagate(window, cx);
    }

    /// Whether the review keys belong to something else: a modal, or the
    /// find panel, where Enter goes to the next match and Esc closes it.
    pub(in crate::workspace) fn keys_elsewhere(&self, window: &Window, cx: &App) -> bool {
        self.modal_open(cx)
            || self.find_panel_focused(window, cx)
            || self.planner_focused(window, cx)
            || self.map_field_focused(window, cx)
    }

    /// A field on the Map (a domain's name or purpose, a sketch's label)
    /// has the keyboard.
    fn map_field_focused(&self, window: &Window, cx: &App) -> bool {
        self.map_showing()
            .is_some_and(|view| view.read(cx).editing(window, cx))
    }

    /// Tab accepts a change under review, or moves on in a snippet.
    pub(in crate::workspace) fn on_accept_tab(
        &mut self,
        _: &IndentInline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.keys_elsewhere(window, cx)
            && (self.settle_agent_request(true, window, cx)
                || self.accept_preview(window, cx)
                || self.snippet_step(true, cx))
        {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    /// Shift-Tab goes back in a snippet.
    pub(in crate::workspace) fn on_shift_tab(
        &mut self,
        _: &OutdentInline,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.keys_elsewhere(window, cx) && self.snippet_step(false, cx) {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    pub(in crate::workspace) fn on_accept_indent(
        &mut self,
        _: &Indent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.accept_or_propagate(window, cx);
    }

    /// Enter or Tab: accept the change under review, or allow what the
    /// agent asks to do.
    fn accept_or_propagate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.keys_elsewhere(window, cx)
            && (self.settle_agent_request(true, window, cx) || self.accept_preview(window, cx))
        {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    /// Cmd+Z during review rejects the change.
    pub(in crate::workspace) fn on_undo(
        &mut self,
        _: &Undo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.keys_elsewhere(window, cx) && self.reject_preview(window, cx) {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    /// Esc rejects a pending edit or what the agent asks to do, dismisses a
    /// note, goes back to an agent edit waiting in another tab, stops the
    /// agent's turn, or closes the conversation.
    /// With the palette open, the palette handles Esc itself.
    pub(in crate::workspace) fn on_escape(
        &mut self,
        _: &Escape,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.map_field_focused(window, cx)
            && let Some(view) = self.map_showing().cloned()
        {
            // A domain's name or purpose: left as it was.
            view.update(cx, |view, cx| {
                view.finish_edit(false, cx);
                view.finish_sketch_edit(false, cx);
            });
            view.focus_handle(cx).focus(window, cx);
            cx.stop_propagation();
        } else if self.modal_open(cx) && self.editor().state().focus_handle(cx).is_focused(window) {
            // Focus went back to the code (e.g. a click outside the form or
            // the input), so the floating window can't hear Escape.
            self.close_modals(cx);
            cx.stop_propagation();
            cx.notify();
        } else if self.keys_elsewhere(window, cx) || self.quick_fix_open(cx) {
            // The Quick Fix list closes itself.
            cx.propagate();
        } else if self.reject_preview(window, cx)
            || self.settle_agent_request(false, window, cx)
            || self.close_hunk_popup(cx)
            || self.close_find_panel(cx)
            || self.close_code_hover(cx)
            || self.close_problem(cx)
        {
            cx.stop_propagation();
        } else if self.note.take().is_some() {
            cx.stop_propagation();
            cx.notify();
        } else if self.code_takes_escape(cx) {
            // The first Escape closes the completion list, the next leaves
            // the snippet; neither stops the agent.
            let listing = self.editor().state().read(cx).completion_menu_state().open;
            if !listing {
                self.end_snippet();
            }
            cx.propagate();
        } else if self.escape_agent(window, cx) {
            cx.stop_propagation();
        } else {
            cx.propagate();
        }
    }

    /// The code has a completion list open or a snippet being filled in,
    /// which Esc closes before anything else.
    fn code_takes_escape(&self, cx: &App) -> bool {
        self.editor().state().read(cx).completion_menu_state().open
            || self
                .tabs
                .get(self.active)
                .is_some_and(|tab| tab.snippet.is_some())
    }
}
