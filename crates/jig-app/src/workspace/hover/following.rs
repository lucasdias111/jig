//! What the hover follows: the pointer and the cursor, what's typed, and the server's answers.

use super::*;

impl CodeHover {
    pub(super) fn new(
        workspace: WeakEntity<Workspace>,
        editor: Entity<EditorState>,
        problems: Entity<ProblemHover>,
        cx: &mut Context<Self>,
    ) -> Self {
        let observe_editor = cx.observe(&editor, |this, _, cx| this.editor_changed(cx));
        // The palette or a run opening may cover the panels.
        let observe_workspace = match workspace.upgrade() {
            Some(workspace) => cx.observe(&workspace, |_, _, cx| cx.notify()),
            None => Subscription::new(|| {}),
        };
        let fixes = cx.new(|_| PanelFixes::new(workspace.clone(), editor.entity_id()));
        Self {
            cursor: editor.read(cx).cursor(),
            workspace,
            editor,
            problems,
            pinned: None,
            last: None,
            held: false,
            leaving: None,
            fixes,
            call: None,
            hover_task: Task::ready(()),
            signature_task: Task::ready(()),
            _observe_editor: observe_editor,
            _observe_workspace: observe_workspace,
        }
    }

    pub(super) fn server(&self, cx: &App) -> Option<(Arc<Client>, String)> {
        self.workspace
            .upgrade()?
            .read(cx)
            .hover_server(self.editor.entity_id(), cx)
    }

    /// The cursor moved or the text changed: put away what no longer
    /// applies, and follow the call to the parameter now being typed.
    fn editor_changed(&mut self, cx: &mut Context<Self>) {
        self.follow_pointer(cx);
        let cursor = self.editor.read(cx).cursor();
        if cursor == self.cursor {
            return;
        }
        self.cursor = cursor;
        if self
            .pinned
            .as_ref()
            .is_some_and(|pinned| pinned.at != cursor)
        {
            self.pinned = None;
            self.hover_task = Task::ready(());
            cx.notify();
        }
        if let Some(call) = &self.call {
            let text = self.editor.read(cx).text().to_string();
            if hover::enclosing_call(&text, cursor) == Some(call.open) {
                self.ask_signature(None, cx);
            } else {
                self.close_signature(cx);
            }
        }
    }

    /// Keep what the pointer's hover shows, and let it linger as the
    /// pointer leaves.
    fn follow_pointer(&mut self, cx: &mut Context<Self>) {
        self.follow(cx);
        self.tell_problems(cx);
    }

    fn follow(&mut self, cx: &mut Context<Self>) {
        if self.held {
            return;
        }
        match self.pointer_shown(cx) {
            Some(shown) => {
                self.last = Some(shown);
                self.leaving = None;
            }
            None if self.last.is_some() && self.leaving.is_none() => {
                self.leaving = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(LINGER).await;
                    this.update(cx, |this, cx| {
                        this.leaving = None;
                        if !this.held {
                            // Following again would only linger again.
                            this.last = this.pointer_shown(cx);
                            this.tell_problems(cx);
                            cx.notify();
                        }
                    })
                    .ok();
                }));
            }
            None => {}
        }
    }

    /// The problem panel stays out of the way while this shows the problem.
    fn tell_problems(&self, cx: &mut Context<Self>) {
        let showing = self.last.is_some()
            && (self.held || self.leaving.is_some() || self.pointer_shown(cx).is_some());
        self.problems
            .update(cx, |problems, cx| problems.set_under_hover(showing, cx));
    }

    pub(super) fn set_held(&mut self, held: bool, cx: &mut Context<Self>) {
        if self.held == held {
            return;
        }
        self.held = held;
        if !held {
            self.follow_pointer(cx);
        }
        cx.notify();
    }

    /// `text` was typed: `(` and `,` ask for the call's signature, `)` puts
    /// it away; anything puts away the hover.
    pub(super) fn typed(&mut self, text: &str, _window: &mut Window, cx: &mut Context<Self>) {
        self.forget_pointer(cx);
        if self.pinned.take().is_some() {
            self.hover_task = Task::ready(());
            cx.notify();
        }
        match text {
            ")" => self.close_signature(cx),
            "(" | "," => self.ask_signature(Some(text), cx),
            _ => {}
        }
    }

    fn close_signature(&mut self, cx: &mut Context<Self>) {
        self.signature_task = Task::ready(());
        if self.call.take().is_some() {
            cx.notify();
        }
    }

    /// Ask for the signature of the call the cursor is in, typing
    /// `trigger`, or without one, after the cursor moved within it.
    fn ask_signature(&mut self, trigger: Option<&str>, cx: &mut Context<Self>) {
        let state = self.editor.read(cx);
        let source = state.text().to_string();
        let cursor = state.cursor();
        let Some(open) = hover::enclosing_call(&source, cursor) else {
            self.close_signature(cx);
            return;
        };
        let Some((client, uri)) = self.server(cx) else {
            return;
        };
        client.change(&uri, &source);
        let encoding = client.encoding();
        let mut params =
            lsp::text_document_position(&uri, lsp::position(&source, cursor, encoding));
        params["context"] = match trigger {
            Some(trigger) => json!({
                "triggerKind": 2,
                "triggerCharacter": trigger,
                "isRetrigger": self.call.is_some(),
            }),
            None => json!({"triggerKind": 3, "isRetrigger": self.call.is_some()}),
        };
        let request = client.request("textDocument/signatureHelp", params);
        self.signature_task = cx.spawn(async move |this, cx| {
            let answer = with_timeout(request, SIGNATURE_TIMEOUT, cx).await;
            let signature = answer.as_ref().and_then(hover::signature);
            this.update(cx, |this, cx| {
                // Only while the cursor is still in that call.
                let state = this.editor.read(cx);
                let still =
                    hover::enclosing_call(&state.text().to_string(), state.cursor()) == Some(open);
                this.call = signature
                    .filter(|_| still)
                    .map(|signature| Call { open, signature });
                cx.notify();
            })
            .ok();
        });
    }

    /// Show what the server says about the name at the cursor, under the
    /// problems there.
    pub(super) fn show_at_cursor(
        &mut self,
        server: Option<(Arc<Client>, String)>,
        cx: &mut Context<Self>,
    ) {
        let state = self.editor.read(cx);
        let cursor = state.cursor();
        let source = state.text().to_string();
        let problems: Vec<DiagnosticEntry> = state
            .diagnostics()
            .map(|set| {
                set.iter()
                    .filter(|e| e.range.start <= cursor && cursor <= e.range.end)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let name = jig_lsp::definitions::name_at(&source, cursor);
        let range = name.clone().unwrap_or(cursor..cursor);
        let pin = move |this: &mut Self,
                        markdown: Option<SharedString>,
                        range: Range<usize>,
                        cx: &mut Context<Self>| {
            if markdown.is_none() && problems.is_empty() {
                return;
            }
            // The problem shows here, not twice.
            this.problems.update(cx, |problems, cx| problems.unpin(cx));
            this.pinned = Some(Pinned {
                at: cursor,
                range,
                markdown,
                problems,
            });
            cx.notify();
        };
        let Some((client, uri)) = server.filter(|_| name.is_some()) else {
            pin(self, None, range, cx);
            return;
        };
        let answer = ask_hover(&client, &uri, &source, cursor);
        self.hover_task = cx.spawn(async move |this, cx| {
            let found = answer(cx).await;
            this.update(cx, |this, cx| {
                if this.editor.read(cx).cursor() != cursor {
                    return;
                }
                let (markdown, at) = match found {
                    Some((markdown, at)) => (Some(markdown.into()), at.unwrap_or(range)),
                    None => (None, range),
                };
                pin(this, markdown, at, cx);
            })
            .ok();
        });
    }

    /// Drop what the pointer's hover showed, at once.
    fn forget_pointer(&mut self, cx: &mut Context<Self>) {
        self.last = None;
        self.leaving = None;
        self.held = false;
        self.tell_problems(cx);
    }

    pub(in crate::workspace) fn close(&mut self, cx: &mut Context<Self>) -> bool {
        let hovering = self.editor.read(cx).hovered_symbol().is_some();
        if hovering {
            self.editor
                .update(cx, |editor, cx| editor.clear_hover_state(cx));
        }
        let lingering = self.last.is_some() && (self.held || self.leaving.is_some());
        self.forget_pointer(cx);
        let closed =
            self.pinned.take().is_some() | self.call.take().is_some() | hovering | lingering;
        self.hover_task = Task::ready(());
        self.signature_task = Task::ready(());
        if closed {
            cx.notify();
        }
        closed
    }

    /// What the pointer's hover shows: the symbol under it, with the
    /// problem there.
    fn pointer_shown(&self, cx: &App) -> Option<Shown> {
        let symbol = self.editor.read(cx).hovered_symbol()?;
        let markdown = match &symbol.hover.contents {
            HoverContents::Markup(markup) => markup.value.clone(),
            other => hover::markdown(&serde_json::to_value(other).ok()?)?,
        };
        // The problem lingers as the pointer leaves it, as its own panel
        // does.
        let problems = self
            .problems
            .read(cx)
            .pointed_at(cx)
            .map(|entry| vec![(*entry).clone()])
            .unwrap_or_default();
        Some((symbol.symbol_range.clone(), Some(markdown.into()), problems))
    }

    /// What the hover shows: the pointer's first, else the keys'.
    pub(super) fn shown(&self, cx: &App) -> Option<Shown> {
        if (self.held || (self.leaving.is_some() && self.pointer_shown(cx).is_none()))
            && let Some(last) = &self.last
        {
            return Some(last.clone());
        }
        if let Some(shown) = self.pointer_shown(cx) {
            return Some(shown);
        }
        let pinned = self.pinned.as_ref()?;
        Some((
            pinned.range.clone(),
            pinned.markdown.clone(),
            pinned.problems.clone(),
        ))
    }

    /// The hover's Markdown and problems' messages, if it's showing.
    #[cfg(test)]
    pub(in crate::workspace) fn hover_text(
        &self,
        cx: &App,
    ) -> Option<(Option<String>, Vec<String>)> {
        self.shown(cx).map(|(_, markdown, problems)| {
            (
                markdown.map(|m| m.to_string()),
                problems.iter().map(|e| e.message.to_string()).collect(),
            )
        })
    }

    #[cfg(test)]
    pub(in crate::workspace) fn signature(&self) -> Option<(String, Option<String>)> {
        self.call.as_ref().map(|call| {
            let signature = &call.signature;
            (
                signature.label.clone(),
                signature
                    .active
                    .clone()
                    .map(|range| signature.label[range].to_string()),
            )
        })
    }

    pub(super) fn hidden(&self, cx: &App) -> bool {
        self.workspace
            .upgrade()
            .is_none_or(|workspace| workspace.read(cx).hides_code_hover(cx))
    }
}
