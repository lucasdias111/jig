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
use jig_editing::hover::{self, Signature};
use jig_lsp::lsp::{self, Client};

mod following;
mod view;

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
    pub(super) fn hides_code_hover(&self, cx: &App) -> bool {
        self.modal_open(cx) || self.previewing()
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
    fn hover_server(&self, editor: EntityId, cx: &App) -> Option<(Arc<Client>, String)> {
        if self.modal_open(cx) || self.previewing() {
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
        let server = self.hover_server(self.tab().id(), cx);
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
        if jig_lsp::definitions::name_at(&source, offset).is_none() {
            return Task::ready(Ok(None));
        }
        let Some((client, uri)) = self
            .workspace
            .upgrade()
            .map(|workspace| workspace.read(cx))
            .filter(|workspace| !workspace.pointer_on_panel(self.editor, cx))
            .and_then(|workspace| workspace.hover_server(self.editor, cx))
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
