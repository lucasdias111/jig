//! Suggestions while typing a name, in a list under the cursor; Enter takes
//! one, Esc or anything else dismisses it.
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
use lsp_types::{CompletionContext, CompletionResponse, CompletionTextEdit, TextEdit};

use super::Workspace;
use super::definitions::with_timeout;
use super::snippets::{Offer, Offers};
use crate::completions::{self, Completion};
use crate::lsp::{self, Client};

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
    crate::settings::get(cx).editor.autocomplete
}

impl CompletionProvider for Completions {
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<CompletionResponse>> {
        if !enabled(cx) {
            return Task::ready(Ok(CompletionResponse::Array(Vec::new())));
        }
        let Some(info) = self.tab_info(cx) else {
            return Task::ready(Ok(CompletionResponse::Array(Vec::new())));
        };
        let rope = text.clone();
        let source = text.to_string();
        let snippets = completions::from_snippets(
            &source,
            offset,
            info.language,
            &crate::snippets::for_language(info.language),
            &info.unit,
        );
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
                None => completions::from_words(&source, offset),
            };
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
                client
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
