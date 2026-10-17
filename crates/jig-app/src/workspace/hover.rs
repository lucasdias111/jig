//! What the language server says about the code, where you're looking:
//! resting the pointer on a name (or the Show Hover keys) shows its type and
//! docs in a panel below it, under any problem there; typing `(` or `,` in
//! a call shows its signature above the cursor, the parameter being typed
//! in bold, until `)`, Esc, or the cursor leaves the call.
//!
//! GPUI Kit's editor follows the pointer and asks after a delay; this gives
//! it a [`HoverProvider`] and draws the answer itself (a vendored-editor
//! patch), in Jig's panels. Without a server nothing shows.

use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use gpui_kit::base::input::{DiagnosticEntry, TypedHandler};
use gpui_kit::component::input::{EditorState, HoverProvider, Rope, RopeExt as _};
use gpui_kit::component::text::{TextView, TextViewStyle};
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind};
use serde_json::json;

use super::definitions::with_timeout;
use super::diagnostics::{LINGER, ProblemHover, problem_row};
use super::quick_fix::PanelFixes;
use super::{ShowHover, Workspace};
use crate::hover::{self, Signature};
use crate::lsp::{self, Client};

/// How long the pointer rests on a name before the server is asked.
const HOVER_DELAY: Duration = Duration::from_millis(500);
/// Past this, the answer is no longer what the pointer is on.
const HOVER_TIMEOUT: Duration = Duration::from_secs(3);
/// Past this, typing has moved on.
const SIGNATURE_TIMEOUT: Duration = Duration::from_secs(2);
const PANEL_MAX_WIDTH: f32 = 520.;
const HOVER_MAX_HEIGHT: f32 = 320.;
/// Above the agent's conversation and a note, which are drawn
/// after: the pointer is on the code, so what it asks about comes first.
const PANEL_PRIORITY: usize = 1;

/// Give a tab's editor hover and signature help. Returns the view that
/// shows them.
pub(super) fn install(
    state: &Entity<EditorState>,
    problems: &Entity<ProblemHover>,
    cx: &mut Context<Workspace>,
) -> Entity<CodeHover> {
    let workspace = cx.entity().downgrade();
    let view = cx.new(|cx| CodeHover::new(workspace.clone(), state.clone(), problems.clone(), cx));
    let provider = Rc::new(Hovers {
        workspace,
        editor: state.entity_id(),
    });
    let typed_view = view.downgrade();
    // The editor is mid-update when it says; hear it once it's done.
    let on_typed: TypedHandler = Rc::new(move |text, window, cx| {
        let view = typed_view.clone();
        let text = text.to_string();
        window.defer(cx, move |window, cx| {
            view.update(cx, |view, cx| view.typed(&text, window, cx))
                .ok();
        });
    });
    state.update(cx, |state, _| {
        let lsp = state.lsp_mut();
        lsp.hover_provider = Some(provider);
        lsp.hover_delay = HOVER_DELAY;
        lsp.own_hover_popover = true;
        lsp.on_typed = Some(on_typed);
    });
    view
}

impl Workspace {
    /// Whether the palette or another sheet is open, or a change is under
    /// review, which the panels would get in the way of. The agent's
    /// conversation and a note don't: the panels show over them.
    pub(super) fn hides_code_hover(&self) -> bool {
        self.modal_open() || self.previewing()
    }

    /// Whether the pointer is over a problem or hover panel of the tab of
    /// `editor`, which keeps the hover of the code under it away.
    fn pointer_on_panel(&self, editor: EntityId, cx: &App) -> bool {
        self.tabs
            .iter()
            .find(|tab| tab.id() == editor)
            .is_some_and(|tab| tab.problems.hover.read(cx).is_held() || tab.hover.read(cx).held)
    }

    /// The tab of the editor `editor`, and its server and URI there.
    fn hover_server(&self, editor: EntityId) -> Option<(Arc<Client>, String)> {
        if self.modal_open() || self.previewing() {
            return None;
        }
        let tab = self
            .tabs
            .iter()
            .find(|tab| tab.editor.state().entity_id() == editor)?;
        let lsp = tab.lsp.as_ref()?;
        Some((lsp.client.clone(), lsp.uri.clone()))
    }

    /// Show Hover: what the server says about the name at the cursor, and
    /// any problem there.
    pub(super) fn show_hover(
        &mut self,
        _: &ShowHover,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home || self.keys_elsewhere(window, cx) {
            return;
        }
        let server = self.hover_server(self.tab().id());
        let hover = self.tab().hover.clone();
        hover.update(cx, |hover, cx| hover.show_at_cursor(server, cx));
    }

    /// Esc: put away the hover and the signature.
    pub(super) fn close_code_hover(&mut self, cx: &mut Context<Self>) -> bool {
        if self.home {
            return false;
        }
        self.tab()
            .hover
            .clone()
            .update(cx, |hover, cx| hover.close(cx))
    }
}

struct Hovers {
    workspace: WeakEntity<Workspace>,
    /// The editor this answers for. Only its id: the editor is mid-update
    /// whenever it asks.
    editor: EntityId,
}

impl HoverProvider for Hovers {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<Hover>>> {
        let source = text.to_string();
        // Not on a name: nothing to ask about.
        if crate::definitions::name_at(&source, offset).is_none() {
            return Task::ready(Ok(None));
        }
        let Some((client, uri)) = self
            .workspace
            .upgrade()
            .map(|workspace| workspace.read(cx))
            .filter(|workspace| !workspace.pointer_on_panel(self.editor, cx))
            .and_then(|workspace| workspace.hover_server(self.editor))
        else {
            return Task::ready(Ok(None));
        };
        let rope = text.clone();
        let answer = ask_hover(&client, &uri, &source, offset);
        cx.spawn(async move |cx| {
            let Some((markdown, range)) = answer(cx).await else {
                return Ok(None);
            };
            Ok(Some(Hover {
                contents: HoverContents::Markup(MarkupContent {
                    kind: MarkupKind::Markdown,
                    value: markdown,
                }),
                range: range.map(|range| {
                    lsp_types::Range::new(
                        rope.offset_to_position(range.start),
                        rope.offset_to_position(range.end),
                    )
                }),
            }))
        })
    }
}

/// Ask the server about the name at `offset` in `source`. Answers with its
/// Markdown and the name's range in `source`, if it gave one.
fn ask_hover(
    client: &Arc<Client>,
    uri: &str,
    source: &str,
    offset: usize,
) -> impl AsyncFnOnce(&AsyncApp) -> Option<(String, Option<Range<usize>>)> + use<> {
    client.change(uri, source);
    let encoding = client.encoding();
    let request = client.request(
        "textDocument/hover",
        lsp::text_document_position(uri, lsp::position(source, offset, encoding)),
    );
    let source = source.to_string();
    async move |cx: &AsyncApp| {
        let answer = with_timeout(request, HOVER_TIMEOUT, cx).await?;
        let markdown = hover::markdown(answer.get("contents")?)?;
        let range = answer
            .get("range")
            .and_then(|range| serde_json::from_value(range.clone()).ok())
            .map(|range| lsp::range(&source, range, encoding));
        Some((markdown, range))
    }
}

/// The hover the keys showed, kept while the cursor stays put.
struct Pinned {
    at: usize,
    range: Range<usize>,
    markdown: Option<SharedString>,
    problems: Vec<DiagnosticEntry>,
}

/// What the hover shows: the symbol's range, the server's Markdown and the
/// problems there.
type Shown = (Range<usize>, Option<SharedString>, Vec<DiagnosticEntry>);

/// The signature shown, and where the call it's for opens.
struct Call {
    open: usize,
    signature: Signature,
}

/// A tab's hover and signature panels. A view of its own, so the pointer
/// and cursor moving repaint only this.
pub(super) struct CodeHover {
    workspace: WeakEntity<Workspace>,
    editor: Entity<EditorState>,
    problems: Entity<ProblemHover>,
    pinned: Option<Pinned>,
    /// What the pointer's hover last showed. It stays a moment after the
    /// pointer leaves, and while the pointer is over the panel, so its
    /// fixes and text can be reached.
    last: Option<Shown>,
    /// The pointer is over the panel.
    held: bool,
    /// The pointer has left; `last` goes when this finishes.
    leaving: Option<Task<()>>,
    fixes: Entity<PanelFixes>,
    call: Option<Call>,
    /// The cursor when last looked at, to tell when it moves.
    cursor: usize,
    hover_task: Task<()>,
    signature_task: Task<()>,
    _observe_editor: Subscription,
    _observe_workspace: Subscription,
}

impl CodeHover {
    fn new(
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

    fn server(&self, cx: &App) -> Option<(Arc<Client>, String)> {
        self.workspace
            .upgrade()?
            .read(cx)
            .hover_server(self.editor.entity_id())
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

    fn set_held(&mut self, held: bool, cx: &mut Context<Self>) {
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
    fn typed(&mut self, text: &str, _window: &mut Window, cx: &mut Context<Self>) {
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
    fn show_at_cursor(&mut self, server: Option<(Arc<Client>, String)>, cx: &mut Context<Self>) {
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
        let name = crate::definitions::name_at(&source, cursor);
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

    pub(super) fn close(&mut self, cx: &mut Context<Self>) -> bool {
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
    fn shown(&self, cx: &App) -> Option<Shown> {
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
    pub(super) fn hover_text(&self, cx: &App) -> Option<(Option<String>, Vec<String>)> {
        self.shown(cx).map(|(_, markdown, problems)| {
            (
                markdown.map(|m| m.to_string()),
                problems.iter().map(|e| e.message.to_string()).collect(),
            )
        })
    }

    #[cfg(test)]
    pub(super) fn signature(&self) -> Option<(String, Option<String>)> {
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

    fn hidden(&self, cx: &App) -> bool {
        self.workspace
            .upgrade()
            .is_none_or(|workspace| workspace.read(cx).hides_code_hover())
    }

    fn render_hover(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (range, markdown, problems) = self.shown(cx)?;
        let state = self.editor.read(cx);
        let at = state
            .range_to_bounds(&range)
            .or_else(|| state.range_to_bounds(&(range.start..range.start)))?;
        // Below the name, or above it low in the window.
        let above = at.top() > window.viewport_size().height * 0.6;
        let (position, corner) = if above {
            (
                point(at.left() - px(10.), at.top() - px(4.)),
                Anchor::BottomLeft,
            )
        } else {
            (
                point(at.left() - px(10.), at.bottom() + px(4.)),
                Anchor::TopLeft,
            )
        };
        if self.hidden(cx) {
            return None;
        }
        if let Some(first) = problems.first() {
            self.fixes
                .update(cx, |fixes, cx| fixes.offer_for(first, cx));
        }
        let theme = cx.theme();
        let has_problems = !problems.is_empty();
        let docs = markdown.map(|markdown| {
            let code = StyleRefinement::default()
                .bg(theme.transparent)
                .p_0()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(12.));
            TextView::markdown("hover-docs", markdown)
                .style(
                    TextViewStyle::default()
                        .paragraph_gap(rems(0.5))
                        .code_block(code),
                )
                .selectable(true)
        });
        let panel = jig_commands::surface::panel(cx)
            .id("code-hover")
            .debug_selector(|| "code-hover".into())
            .max_w(px(PANEL_MAX_WIDTH))
            .max_h(px(HOVER_MAX_HEIGHT))
            .overflow_y_scroll()
            .px_2p5()
            .py_1p5()
            .text_size(px(12.5))
            .on_hover(cx.listener(|this, held: &bool, _, cx| this.set_held(*held, cx)))
            .child(
                v_flex()
                    .gap_1p5()
                    .children(problems.iter().map(|entry| problem_row(entry, cx)))
                    .when(has_problems, |this| this.child(self.fixes.clone()))
                    .when(has_problems && docs.is_some(), |this| {
                        this.child(div().h(px(1.)).bg(theme.border))
                    })
                    .children(docs),
            );
        Some(
            deferred(
                anchored()
                    .anchor(corner)
                    .position(position)
                    .snap_to_window_with_margin(px(8.))
                    .child(panel),
            )
            .with_priority(PANEL_PRIORITY)
            .into_any_element(),
        )
    }

    fn render_signature(&self, cx: &App) -> Option<AnyElement> {
        let call = self.call.as_ref()?;
        let state = self.editor.read(cx);
        let cursor = state.cursor();
        let at = state.range_to_bounds(&(cursor..cursor))?;
        let position = point(at.left() - px(10.), at.top() - px(4.));
        if self.hidden(cx) {
            return None;
        }
        let theme = cx.theme();
        let signature = &call.signature;
        let emphasis = HighlightStyle {
            font_weight: Some(FontWeight::BOLD),
            color: Some(theme.link),
            ..Default::default()
        };
        let label = StyledText::new(signature.label.clone())
            .with_highlights(signature.active.clone().map(|range| (range, emphasis)));
        let panel = jig_commands::surface::panel(cx)
            .id("signature-help")
            .debug_selector(|| "signature-help".into())
            .max_w(px(PANEL_MAX_WIDTH))
            .px_2p5()
            .py_1p5()
            .text_size(px(12.5))
            .child(
                h_flex()
                    .items_start()
                    .gap_2()
                    .child(
                        div()
                            .min_w_0()
                            .font_family(theme.mono_font_family.clone())
                            .child(label),
                    )
                    .when_some(signature.overload, |this, (n, of)| {
                        this.child(
                            div()
                                .flex_none()
                                .text_size(px(11.))
                                .text_color(theme.muted_foreground)
                                .child(format!("{n}/{of}")),
                        )
                    }),
            )
            .when_some(signature.documentation.clone(), |this, docs| {
                this.child(
                    div()
                        .mt_0p5()
                        .text_size(px(11.5))
                        .text_color(theme.muted_foreground)
                        .child(docs),
                )
            });
        Some(
            deferred(
                anchored()
                    .anchor(Anchor::BottomLeft)
                    .position(position)
                    .snap_to_window_with_margin(px(8.))
                    .child(panel),
            )
            .with_priority(PANEL_PRIORITY)
            .into_any_element(),
        )
    }
}

impl Render for CodeHover {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .children(self.render_hover(window, cx))
            .children(self.render_signature(cx))
    }
}
