//! Cmd+click (or F12) on a name: go to where it's declared, in this file or
//! another; on a declaration, list its references in Find in Files.
//!
//! GPUI Kit's editor does the Cmd+hover underline and the click; this gives
//! it a [`DefinitionProvider`] and takes over showing a target that isn't in
//! the current file. The file's language server answers when there is one;
//! otherwise, or when it has no answer (say, while it's still indexing),
//! [`crate::definitions`] guesses.

use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use futures::future::{Either, select};
use gpui_kit::component::input::{DefinitionProvider, EditorState, Rope, RopeExt as _};
use gpui_kit::*;
use jig_editor::EditorHandle;
use lsp_types::{LocationLink, ShowDocumentParams, Uri};
use serde_json::{Value, json};

use super::Workspace;
use super::tabs::canonical;
use crate::definitions::{self, Target};
use crate::languages::language_for;
use crate::lsp::{self, Client};
use crate::project_search::{self, SearchOptions, SearchResults};

/// The URI of a "show usages of this name" target: the name, then the
/// offset it was clicked at.
const USAGES_SCHEME: &str = "jig-usages:";
/// Files larger than this aren't searched for declarations.
const MAX_FILE_BYTES: u64 = 1024 * 1024;
/// How long to wait for a language server before guessing instead.
const DEFINITION_TIMEOUT: Duration = Duration::from_secs(3);
/// References can take a server longer; past this, search the text.
const REFERENCES_TIMEOUT: Duration = Duration::from_secs(10);

/// Give a tab's editor go-to-definition.
pub(super) fn install(state: &Entity<EditorState>, cx: &mut Context<Workspace>) {
    let workspace = cx.entity().downgrade();
    let editor = state.entity_id();
    let provider = Rc::new(Definitions {
        workspace: workspace.clone(),
        editor,
    });
    let show_document = Rc::new(
        move |params: &ShowDocumentParams, window: &mut Window, cx: &mut App| {
            show_document(&workspace, editor, params, window, cx)
        },
    );
    state.update(cx, |state, _| {
        let lsp = state.lsp_mut();
        lsp.definition_provider = Some(provider);
        lsp.show_document = Some(show_document);
    });
}

struct Definitions {
    workspace: WeakEntity<Workspace>,
    /// The editor this answers for. Only its id: the editor is mid-update
    /// whenever it asks.
    editor: EntityId,
}

/// What the provider needs from the workspace about its tab.
struct TabInfo {
    path: Option<PathBuf>,
    language: &'static str,
    root: Option<PathBuf>,
    files: Option<Arc<Vec<String>>>,
    languages: crate::settings::LanguageSettings,
    lsp: Option<(Arc<Client>, String)>,
}

impl Definitions {
    fn tab_info(&self, cx: &App) -> Option<TabInfo> {
        let workspace = self.workspace.upgrade()?.read(cx);
        let tab = workspace
            .tabs
            .iter()
            .find(|tab| tab.editor.state().entity_id() == self.editor)?;
        let root = workspace.project_root(cx);
        Some(TabInfo {
            path: tab.document.path.clone(),
            language: tab.document.language(&workspace.settings.languages),
            files: root
                .as_deref()
                .and_then(|root| workspace.cached_files(root)),
            root,
            languages: workspace.settings.languages.clone(),
            lsp: tab
                .lsp
                .as_ref()
                .map(|lsp| (lsp.client.clone(), lsp.uri.clone())),
        })
    }
}

impl DefinitionProvider for Definitions {
    fn definitions(
        &self,
        text: &Rope,
        offset: usize,
        _window: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Vec<LocationLink>>> {
        let source = text.to_string();
        let Some(range) = definitions::name_at(&source, offset) else {
            return Task::ready(Ok(Vec::new()));
        };
        let Some(info) = self.tab_info(cx) else {
            return Task::ready(Ok(Vec::new()));
        };
        let origin = kit_range(text, &range);
        let workspace = self.workspace.clone();
        let Some((client, uri)) = info.lsp.clone() else {
            return cx.spawn(async move |cx| {
                Ok(guess(source, range, origin, info, workspace, cx).await)
            });
        };

        let encoding = client.encoding();
        let position = lsp::position(&source, range.start, encoding);
        let request = client.request(
            "textDocument/definition",
            lsp::text_document_position(&uri, position),
        );
        cx.spawn(async move |cx| {
            let response = with_timeout(request, DEFINITION_TIMEOUT, cx).await;
            let found = response.as_ref().map(lsp::locations).unwrap_or_default();
            let Some((path, target)) = found.first().cloned() else {
                return Ok(guess(source, range, origin, info, workspace, cx).await);
            };
            // On the declaration itself: its references instead.
            let on_declaration = found.iter().any(|(path, target)| {
                info.path
                    .as_deref()
                    .is_some_and(|here| canonical(here) == canonical(path))
                    && lsp::range(&source, *target, encoding).contains(&range.start)
            });
            if on_declaration {
                return Ok(vec![usages_link(&source, range, origin)]);
            }
            let text = workspace
                .read_with(cx, |workspace, cx| workspace.buffer_text(&path, cx))
                .ok()
                .flatten()
                .or_else(|| std::fs::read_to_string(&path).ok());
            let Some(text) = text else {
                return Ok(Vec::new());
            };
            let target = lsp::range(&text, target, encoding);
            Ok(vec![link(
                origin,
                file_uri(&path),
                kit_range(&Rope::from(text.as_str()), &target),
            )])
        })
    }
}

/// Where the name at `range` is declared, by [`crate::definitions`]'
/// reckoning, for when no language server answers.
async fn guess(
    source: String,
    range: Range<usize>,
    origin: lsp_types::Range,
    info: TabInfo,
    workspace: WeakEntity<Workspace>,
    cx: &mut AsyncApp,
) -> Vec<LocationLink> {
    let rope = Rope::from(source.as_str());
    let here = || {
        info.path
            .as_deref()
            .map_or_else(|| "untitled:".parse::<Uri>().expect("valid URI"), file_uri)
    };
    match definitions::resolve(&source, range.clone(), info.language) {
        Target::Here(target) => vec![link(origin, here(), kit_range(&rope, &target))],
        Target::Usages if info.root.is_some() => vec![usages_link(&source, range, origin)],
        // No project to search: step to the next use in this file.
        Target::Usages => definitions::next_occurrence(&source, range)
            .map(|target| link(origin, here(), kit_range(&rope, &target)))
            .into_iter()
            .collect(),
        Target::Elsewhere => {
            let Some(root) = info.root else {
                return Vec::new();
            };
            let name = source[range.clone()].to_string();
            let member = definitions::is_member_access(&source, range.start);
            let walked = info.files.is_none();
            let search_root = root.clone();
            let current = info.path.clone();
            let (files, found) = cx
                .background_executor()
                .spawn(async move {
                    let files = info
                        .files
                        .unwrap_or_else(|| Arc::new(crate::quick_open::walk(&search_root)));
                    let found = search_files(
                        &search_root,
                        &files,
                        current.as_deref(),
                        &name,
                        info.language,
                        member,
                        &info.languages,
                    );
                    (files, found)
                })
                .await;
            if walked {
                workspace
                    .update(cx, |workspace, _| workspace.remember_files(root, files))
                    .ok();
            }
            found
                .map(|(path, target)| link(origin, file_uri(&path), target))
                .into_iter()
                .collect()
        }
    }
}

/// The first declaration of `name` in the project's files of `language`,
/// other than `current`, with its range in the editor's positions.
fn search_files(
    root: &Path,
    files: &[String],
    current: Option<&Path>,
    name: &str,
    language: &str,
    member: bool,
    languages: &crate::settings::LanguageSettings,
) -> Option<(PathBuf, lsp_types::Range)> {
    let current = current.map(canonical);
    files.iter().find_map(|file| {
        let path = root.join(file);
        if language_for(&path, languages) != language
            || current.as_ref() == Some(&canonical(&path))
            || std::fs::metadata(&path).map_or(true, |m| m.len() > MAX_FILE_BYTES)
        {
            return None;
        }
        let text = std::fs::read_to_string(&path).ok()?;
        let range = definitions::declaration_in_file(&text, name, language, member)?;
        Some((path, kit_range(&Rope::from(text.as_str()), &range)))
    })
}

/// Show a target the editor can't reach on its own. Returns false to let
/// the editor move within its own file.
fn show_document(
    workspace: &WeakEntity<Workspace>,
    editor: EntityId,
    params: &ShowDocumentParams,
    window: &mut Window,
    cx: &mut App,
) -> bool {
    let workspace = workspace.clone();
    let uri = params.uri.as_str();
    if let Some(target) = uri.strip_prefix(USAGES_SCHEME) {
        let Some((name, offset)) = target
            .rsplit_once('/')
            .and_then(|(name, offset)| Some((lsp::decode(name), offset.parse().ok()?)))
        else {
            return true;
        };
        // The editor is mid-update; act once it's done.
        window.defer(cx, move |window, cx| {
            workspace
                .update(cx, |workspace, cx| {
                    workspace.show_usages(&name, offset, window, cx)
                })
                .ok();
        });
        return true;
    }
    let Some(path) = lsp::uri_path(uri) else {
        return false;
    };
    let same_file = workspace.upgrade().is_some_and(|workspace| {
        workspace.read(cx).tabs.iter().any(|tab| {
            tab.editor.state().entity_id() == editor
                && tab
                    .document
                    .path
                    .as_deref()
                    .is_some_and(|open| canonical(open) == canonical(&path))
        })
    });
    if same_file {
        return false;
    }
    let Some(selection) = params.selection else {
        return false;
    };
    window.defer(cx, move |window, cx| {
        workspace
            .update(cx, |workspace, cx| {
                workspace.open_at(&path, selection, window, cx)
            })
            .ok();
    });
    true
}

impl Workspace {
    /// The text of `path` as open in a tab, unsaved edits and all.
    fn buffer_text(&self, path: &Path, cx: &App) -> Option<String> {
        self.tab_for(path).map(|ix| self.tabs[ix].editor.text(cx))
    }

    /// Open `path` with `selection` (in the editor's positions) selected.
    pub(super) fn open_at(
        &mut self,
        path: &Path,
        selection: lsp_types::Range,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_file(path, window, cx);
        if !self
            .document()
            .path
            .as_deref()
            .is_some_and(|open| canonical(open) == canonical(path))
        {
            return;
        }
        let text = Rope::from(self.editor().text(cx).as_str());
        let range =
            text.position_to_offset(&selection.start)..text.position_to_offset(&selection.end);
        self.editor().select(range, cx);
        self.editor().focus(window, cx);
    }

    /// Every use of `name`, clicked at `offset` in the current tab, in Find
    /// in Files: its references from the language server, or failing that,
    /// a search for the word.
    fn show_usages(
        &mut self,
        name: &str,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let options = SearchOptions {
            match_case: true,
            whole_word: true,
            regex: false,
        };
        self.close_find_in_files(cx);
        let Some((root, lsp)) = self.project_root(cx).zip(self.tab().lsp.as_ref()) else {
            self.open_find_in_files(name.to_string(), options, window, cx);
            return;
        };
        let client = lsp.client.clone();
        let encoding = client.encoding();
        let position = lsp::position(&self.editor().text(cx), offset, encoding);
        let mut params = lsp::text_document_position(&lsp.uri, position);
        params["context"] = json!({"includeDeclaration": true});
        let request = client.request("textDocument/references", params);
        let name = name.to_string();
        cx.spawn_in(window, async move |this, cx| {
            let response = with_timeout(request, REFERENCES_TIMEOUT, cx).await;
            let found = response.as_ref().map(lsp::locations).unwrap_or_default();
            // Unsaved edits are what the server answered about.
            let buffers: HashMap<PathBuf, String> = this
                .read_with(cx, |this, cx| {
                    found
                        .iter()
                        .filter_map(|(path, _)| Some((path.clone(), this.buffer_text(path, cx)?)))
                        .collect()
                })
                .unwrap_or_default();
            let results = cx
                .background_executor()
                .spawn(async move { reference_results(&root, found, buffers, encoding) })
                .await;
            this.update_in(cx, |this, window, cx| {
                this.open_find_in_files(name, options, window, cx);
                if results.matches.is_empty() {
                    // Nothing from the server: the word search stands.
                    return;
                }
                if let Some(open) = &this.find_in_files {
                    open.view
                        .update(cx, |view, cx| view.show_results(results, window, cx));
                }
            })
            .ok();
        })
        .detach();
    }
}

/// References as Find in Files rows, by file then position.
fn reference_results(
    root: &Path,
    mut found: Vec<(PathBuf, lsp_types::Range)>,
    buffers: HashMap<PathBuf, String>,
    encoding: lsp::Encoding,
) -> SearchResults {
    found.sort_by(|(a, x), (b, y)| {
        a.cmp(b)
            .then(x.start.line.cmp(&y.start.line))
            .then(x.start.character.cmp(&y.start.character))
    });
    found.dedup();
    let mut results = SearchResults::default();
    let mut texts: HashMap<PathBuf, Option<String>> = HashMap::new();
    let mut last_path = None;
    for (path, range) in found {
        let text = texts.entry(path.clone()).or_insert_with(|| {
            buffers
                .get(&path)
                .cloned()
                .or_else(|| std::fs::read_to_string(&path).ok())
        });
        let Some(text) = text else {
            continue;
        };
        // Servers may hand back the real path for a symlinked one.
        let shown = path
            .strip_prefix(root)
            .ok()
            .map(Path::to_path_buf)
            .or_else(|| {
                let real = canonical(&path);
                real.strip_prefix(canonical(root))
                    .ok()
                    .map(Path::to_path_buf)
            })
            .map_or_else(
                || path.to_string_lossy().into_owned(),
                |relative| relative.to_string_lossy().replace('\\', "/"),
            );
        let found = project_search::line_match(&shown, text, lsp::range(text, range, encoding));
        if last_path.as_ref() != Some(&path) {
            results.files += 1;
            last_path = Some(path);
        }
        results.total_lines += 1;
        if results.matches.len() < project_search::MAX_RESULTS {
            results.matches.push(found);
        }
    }
    results
}

/// What `request` answered, or `None` if it failed or took longer than
/// `timeout`.
pub(super) async fn with_timeout(
    request: futures::channel::oneshot::Receiver<lsp::Response>,
    timeout: Duration,
    cx: &AsyncApp,
) -> Option<Value> {
    let timer = cx.background_executor().timer(timeout);
    match select(request, Box::pin(timer)).await {
        Either::Left((Ok(Ok(value)), _)) => Some(value),
        _ => None,
    }
}

fn link(origin: lsp_types::Range, uri: Uri, target: lsp_types::Range) -> LocationLink {
    LocationLink {
        origin_selection_range: Some(origin),
        target_uri: uri,
        target_range: target,
        target_selection_range: target,
    }
}

/// A link that, followed, shows the usages of the name at `range`.
fn usages_link(source: &str, range: Range<usize>, origin: lsp_types::Range) -> LocationLink {
    let uri = format!(
        "{USAGES_SCHEME}{}/{}",
        lsp::encode(&source[range.clone()]),
        range.start
    );
    link(origin, uri.parse().expect("valid URI"), origin)
}

/// A range in the editor's own positions, which is what it reads links in.
fn kit_range(text: &Rope, range: &Range<usize>) -> lsp_types::Range {
    lsp_types::Range::new(
        text.offset_to_position(range.start),
        text.offset_to_position(range.end),
    )
}

fn file_uri(path: &Path) -> Uri {
    lsp::file_uri(path)
        .parse()
        .expect("percent-encoded paths are valid URIs")
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::reference_results;
    use crate::lsp::Encoding;

    #[test]
    fn references_become_rows() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("b.rs"), "fn b() {\n    alpha();\n}\n").unwrap();
        let at = |line, start, end| {
            lsp_types::Range::new(
                lsp_types::Position::new(line, start),
                lsp_types::Position::new(line, end),
            )
        };
        let found: Vec<(PathBuf, lsp_types::Range)> = vec![
            (root.join("b.rs"), at(1, 4, 9)),
            (root.join("a.rs"), at(0, 7, 12)),
            (root.join("b.rs"), at(1, 4, 9)),
        ];
        // a.rs is open with unsaved text; b.rs is read from disk.
        let buffers = [(root.join("a.rs"), "pub fn alpha() {}\n".to_string())].into();
        let results = reference_results(Path::new(root), found, buffers, Encoding::Utf16);
        let rows: Vec<String> = results
            .matches
            .iter()
            .map(|m| format!("{}:{}: {} {:?}", m.path, m.line + 1, m.text, m.range))
            .collect();
        assert_eq!(
            rows,
            ["a.rs:1: pub fn alpha() {} 7..12", "b.rs:2: alpha(); 13..18"]
        );
        assert_eq!(results.files, 2);
    }
}
