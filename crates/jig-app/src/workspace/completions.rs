//! Suggestions while typing a name, in a list under the cursor; Enter takes
//! one, Esc or anything else dismisses it. Opening a struct literal lists
//! its fields, and Show Completions (⌃Space) lists what fits at the cursor.
//!
//! GPUI Kit's editor shows the list and asks again on each keystroke; this
//! gives it a [`CompletionProvider`]. The file's language server answers
//! when there is one; otherwise, or when it doesn't answer in time,
//! [`crate::completions`] offers other words in the file. Jig's own snippets
//! join either; taking one starts filling it in ([`super::snippets`]).

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use gpui_kit::component::input::{CompletionProvider, EditorState, Rope, RopeExt as _};
use gpui_kit::*;
use lsp_types::{
    CompletionContext, CompletionResponse, CompletionTextEdit, CompletionTriggerKind, TextEdit,
};

use super::definitions::with_timeout;
use super::snippets::{Offer, Offers};
use super::{ShowCompletions, Workspace};
use crate::completions::{self, Completion};
use jig_lsp::lsp::{self, Client};

/// Past this, the server is too slow to keep up with typing; offer words.
const COMPLETION_TIMEOUT: Duration = Duration::from_millis(1500);

/// Give a tab's editor completions. Returns where the snippets it offers
/// are kept.
pub(super) fn install(state: &Entity<EditorState>, cx: &mut Context<Workspace>) -> Offers {
    let offers = Offers::default();
    let provider = Rc::new(Completions {
        workspace: cx.entity().downgrade(),
        editor: state.entity_id(),
        offers: offers.clone(),
    });
    state.update(cx, |state, _| {
        state.lsp_mut().completion_provider = Some(provider);
    });
    offers
}

struct Completions {
    workspace: WeakEntity<Workspace>,
    /// The editor this answers for. Only its id: the editor is mid-update
    /// whenever it asks.
    editor: EntityId,
    offers: Offers,
}

/// What the provider needs from the workspace about its tab.
struct TabInfo {
    language: &'static str,
    /// One level of indentation.
    unit: String,
    /// The tab's language server, and its file's URI there.
    server: Option<(Arc<Client>, String)>,
}

impl Completions {
    fn tab_info(&self, cx: &App) -> Option<TabInfo> {
        let workspace = self.workspace.upgrade()?.read(cx);
        let tab = workspace
            .tabs
            .iter()
            .find(|tab| tab.editor.state().entity_id() == self.editor)?;
        let indentation = tab.indentation;
        Some(TabInfo {
            language: tab.document.language(&workspace.settings.languages),
            unit: if indentation.hard_tabs {
                "\t".into()
            } else {
                " ".repeat(indentation.tab_size)
            },
            server: tab
                .lsp
                .as_ref()
                .map(|lsp| (lsp.client.clone(), lsp.uri.clone())),
        })
    }
}

fn enabled(cx: &App) -> bool {
    jig_settings::get(cx).editor.autocomplete
}

/// Typed in a struct literal, before a field: worth asking the server
/// whether its fields go here. The space or new line after `{` or `,`, not
/// those themselves, so the Enter that starts the next line isn't taken
/// as picking a field.
fn may_start_fields(new_text: &str) -> bool {
    !new_text.is_empty() && new_text.trim_matches([' ', '\t', '\n', '\r']).is_empty()
}

impl Workspace {
    /// Show Completions: what fits at the cursor, whether or not anything
    /// has been typed.
    pub(super) fn show_completions(
        &mut self,
        _: &ShowCompletions,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home || self.previewing() || self.keys_elsewhere(window, cx) {
            return;
        }
        let invoked = CompletionContext {
            trigger_kind: CompletionTriggerKind::INVOKED,
            trigger_character: None,
        };
        self.complete_at_cursor(invoked, window, cx);
    }

    /// Enter started a line: in a struct literal, list its fields there.
    /// The editor doesn't count a new line as typing.
    pub(super) fn completions_after_enter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let typed = CompletionContext {
            trigger_kind: CompletionTriggerKind::TRIGGER_CHARACTER,
            trigger_character: Some("\n".into()),
        };
        self.complete_at_cursor(typed, window, cx);
    }

    /// Ask the tab's completions at the cursor and list them.
    fn complete_at_cursor(
        &mut self,
        trigger: CompletionContext,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = self.editor().state().clone();
        let (provider, text, offset) = {
            let state = state.read(cx);
            let Some(provider) = state.lsp().completion_provider.clone() else {
                return;
            };
            (provider, state.text().clone(), state.cursor())
        };
        let source = text.to_string();
        let start = completions::name_start(&source, offset);
        let typed = source[start..offset].to_string();
        // The provider reads the workspace, which is mid-update here.
        window.defer(cx, move |window, cx| {
            let request = provider.completions(&text, offset, trigger, window, cx);
            cx.spawn(async move |cx| {
                let Ok(response) = request.await else {
                    return;
                };
                let items = match response {
                    CompletionResponse::Array(items) => items,
                    CompletionResponse::List(list) => list.items,
                };
                state.update(cx, |state, cx| {
                    // Moved on while the server thought: not for here.
                    if state.cursor() == offset && !items.is_empty() {
                        state.present_completion_items(start, typed, items, cx);
                    }
                });
            })
            .detach();
        });
    }
}

impl CompletionProvider for Completions {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        let nothing = || Task::ready(Ok(CompletionResponse::Array(Vec::new())));
        let invoked = trigger.trigger_kind == CompletionTriggerKind::INVOKED;
        if !enabled(cx) && !invoked {
            return nothing();
        }
        let Some(info) = self.tab_info(cx) else {
            return nothing();
        };
        let rope = text.clone();
        let source = text.to_string();
        let triggered = |client: &Client| {
            completions::request_context(&source, offset, &client.completion_triggers())["triggerKind"]
                == 2
        };
        // Typed a space or a new line rather than a name or one of
        // the server's own triggers: only a struct literal's fields, if
        // that's where this is.
        let fields_only = !invoked
            && completions::name_start(&source, offset) == offset
            && !info
                .server
                .as_ref()
                .is_some_and(|(client, _)| triggered(client));
        if fields_only
            && (info.server.is_none() || !completions::could_start_a_field(&source, offset))
        {
            return nothing();
        }
        let snippets = if fields_only {
            Vec::new()
        } else {
            completions::from_snippets(
                &source,
                offset,
                info.language,
                &crate::snippets::for_language(info.language),
                &info.unit,
            )
        };
        let offers = self.offers.clone();
        let Some((client, uri)) = info.server else {
            let words = completions::from_words(&source, offset);
            let found = completions::merge(&source, offset, snippets, words);
            return Task::ready(Ok(response(&rope, found, &offers)));
        };
        // The edit that asked for this hasn't reached the server yet: the
        // workspace hears of it only once the editor is done.
        client.change(&uri, &source);
        let encoding = client.encoding();
        let mut params =
            lsp::text_document_position(&uri, lsp::position(&source, offset, encoding));
        params["context"] =
            completions::request_context(&source, offset, &client.completion_triggers());
        let request = client.request("textDocument/completion", params);
        let unit = info.unit;
        cx.spawn(async move |cx| {
            let found = match with_timeout(request, COMPLETION_TIMEOUT, cx).await {
                Some(answer) => completions::from_server(&source, offset, encoding, &answer, &unit),
                None if fields_only => Vec::new(),
                None => completions::from_words(&source, offset),
            };
            if fields_only && !completions::only_fields(&found) {
                return Ok(CompletionResponse::Array(Vec::new()));
            }
            let found = completions::merge(&source, offset, snippets, found);
            Ok(response(&rope, found, &offers))
        })
    }

    fn is_completion_trigger(&self, _offset: usize, new_text: &str, cx: &mut App) -> bool {
        if !enabled(cx) {
            return false;
        }
        let mut chars = new_text.chars();
        // One character typed, not a paste.
        if let (Some(c), None) = (chars.next(), chars.next())
            && completions::is_name_char(c)
        {
            return true;
        }
        self.tab_info(cx)
            .and_then(|info| info.server)
            .is_some_and(|(client, _)| {
                may_start_fields(new_text)
                    || client
                        .completion_triggers()
                        .iter()
                        .any(|trigger| trigger == new_text)
            })
    }
}

/// `found` as the editor takes it: each item carrying its own edit, in the
/// editor's positions. Its snippets go in `offers`.
fn response(text: &Rope, found: Vec<Completion>, offers: &Offers) -> CompletionResponse {
    *offers.borrow_mut() = found
        .iter()
        .filter(|completion| !completion.stops.is_empty())
        .map(|completion| Offer {
            start: completion.replace.start,
            text: completion.new_text.clone(),
            stops: completion.stops.clone(),
        })
        .collect();
    CompletionResponse::Array(
        found
            .into_iter()
            .map(|completion| {
                let range = lsp_types::Range::new(
                    text.offset_to_position(completion.replace.start),
                    text.offset_to_position(completion.replace.end),
                );
                lsp_types::CompletionItem {
                    text_edit: Some(CompletionTextEdit::Edit(TextEdit::new(
                        range,
                        completion.new_text,
                    ))),
                    ..completion.item
                }
            })
            .collect(),
    )
}
