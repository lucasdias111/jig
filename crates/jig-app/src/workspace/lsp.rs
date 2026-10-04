//! Keeping each tab's language server up to date: told when the file opens,
//! changes, is saved and closes.

use std::path::Path;
use std::sync::Arc;

use gpui_kit::*;
use jig_editor::EditorHandle;

use super::Workspace;
use crate::lsp::{self, Client};

/// A tab's file, open in its language server.
pub(super) struct TabLsp {
    pub(super) client: Arc<Client>,
    pub(super) uri: String,
}

impl TabLsp {
    /// Open `path`, reading `text`, in its language's server, starting the
    /// server if need be. `None` when there's no server for it.
    pub(super) fn open(path: &Path, language: &str, text: &str, cx: &mut App) -> Option<Self> {
        let client = lsp::client_for(path, language, cx)?;
        let uri = lsp::file_uri(path);
        client.open(&uri, lsp::language_id(language, path), text);
        Some(Self { client, uri })
    }
}

impl Drop for TabLsp {
    fn drop(&mut self) {
        self.client.close(&self.uri);
    }
}

impl Workspace {
    /// The tab at `ix` was edited.
    pub(super) fn lsp_changed(&self, ix: usize, cx: &App) {
        let tab = &self.tabs[ix];
        if let Some(lsp) = &tab.lsp {
            lsp.client.change(&lsp.uri, &tab.editor.text(cx));
        }
    }

    /// The current tab was saved, maybe under a new name.
    pub(super) fn lsp_saved(&mut self, cx: &mut Context<Self>) {
        let Some(path) = self.document().path.clone() else {
            return;
        };
        let uri = lsp::file_uri(&path);
        if let Some(lsp) = self.tab().lsp.as_ref().filter(|lsp| lsp.uri == uri) {
            lsp.client.save(&lsp.uri);
            return;
        }
        let language = self.document().language(&self.settings.languages);
        let text = self.editor().text(cx);
        let tab = self.tab_mut();
        // The old name closes as the new one opens.
        tab.lsp = None;
        tab.lsp = TabLsp::open(&path, language, &text, cx);
    }
}
