//! Find All References: asking the server, or searching the project, and showing the results.

use super::*;

impl Workspace {
    /// The text of `path` as open in a tab, unsaved edits and all.
    pub(in crate::workspace) fn buffer_text(&self, path: &Path, cx: &App) -> Option<String> {
        self.tab_for(path).map(|ix| self.tabs[ix].editor.text(cx))
    }

    /// Open `path` with `selection` (in the editor's positions) selected.
    pub(in crate::workspace) fn open_at(
        &mut self,
        path: &Path,
        selection: lsp_types::Range,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remember_location(cx);
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
        self.focus_editor(window, cx);
    }

    /// Every use of `name`, clicked at `offset` in the current tab, in Find
    /// in Files: its references from the language server, or failing that,
    /// a search for the word.
    pub(in crate::workspace) fn show_usages(
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
                if let Some(view) = this.find_in_files.read(cx).panel().cloned() {
                    view.update(cx, |view, cx| view.show_results(results, window, cx));
                }
            })
            .ok();
        })
        .detach();
    }
}

/// References as Find in Files rows, by file then position.
pub(super) fn reference_results(
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
