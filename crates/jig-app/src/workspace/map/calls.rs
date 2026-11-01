//! Asking the language server about calls for the Map's Plan, and handing the debugger's frames to it.

use super::*;

impl Workspace {
    /// Ask the language server what function `id` calls and is called by.
    pub(super) fn map_server_calls(
        &mut self,
        view: &Entity<MapView>,
        id: FnId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project) = view.read(cx).project().cloned() else {
            return;
        };
        let function = &project.functions[id];
        let path = project.root.join(&project.files[function.file].path);
        let language = jig_languages::language_for(&path, &self.settings.languages);
        let Some(client) = lsp::client_for(&path, language, cx) else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let uri = lsp::file_uri(&path);
        client.open(&uri, lsp::language_id(language, &path), &text);
        let encoding = client.encoding();
        let position = lsp::position(&text, function.outline.function.name_at, encoding);
        let params = lsp::text_document_position(&uri, position);
        let view = view.downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let mut item = None;
            for attempt in 0..SERVER_ATTEMPTS {
                if attempt > 0 {
                    cx.background_executor().timer(SERVER_RETRY).await;
                }
                let prepare = client.request("textDocument/prepareCallHierarchy", params.clone());
                item = with_timeout(prepare, SERVER_TIMEOUT, cx)
                    .await
                    .and_then(|items| items.as_array()?.first().cloned());
                if item.is_some() {
                    break;
                }
            }
            let answers = match item {
                Some(item) => {
                    let outgoing = client
                        .request("callHierarchy/outgoingCalls", json!({"item": item.clone()}));
                    let outgoing = with_timeout(outgoing, SERVER_TIMEOUT, cx).await;
                    let incoming =
                        client.request("callHierarchy/incomingCalls", json!({"item": item}));
                    let incoming = with_timeout(incoming, SERVER_TIMEOUT, cx).await;
                    Some((outgoing, incoming))
                }
                None => None,
            };
            client.close(&uri);
            let Some((Some(outgoing), incoming)) = answers else {
                return;
            };
            let mut texts = Texts::default();
            let callees = targets(&project, &outgoing, "to", encoding, &mut texts);
            let callers = incoming
                .as_ref()
                .map(|incoming| targets(&project, incoming, "from", encoding, &mut texts))
                .unwrap_or_default()
                .into_iter()
                .filter_map(|target| match target {
                    Target::Function(id) => Some(id),
                    _ => None,
                })
                .collect();
            view.update(cx, |view, cx| view.server_calls(id, callees, callers, cx))
                .ok();
        })
        .detach();
    }

    /// Hand the Map the paused program's call stack, or none while it
    /// runs.
    pub(in crate::workspace) fn refresh_map_debug(&self, cx: &mut App) {
        let Some(pane) = self.map.as_ref() else {
            return;
        };
        let paused = (0..self.tabs.len()).any(|ix| self.execution_offset(ix, cx).is_some());
        let stack: Vec<(PathBuf, u32)> = match self.debug() {
            Some(debug) if paused => debug
                .frames
                .iter()
                .filter_map(|frame| Some((frame.path.clone()?, frame.line.saturating_sub(1))))
                .collect(),
            _ => Vec::new(),
        };
        pane.view.update(cx, |view, cx| view.set_stack(stack, cx));
    }
}

/// Files read once each while the server's answer is mapped.
#[derive(Default)]
pub(super) struct Texts(HashMap<PathBuf, String>);

impl Texts {
    pub(super) fn of(&mut self, path: &Path) -> &str {
        self.0
            .entry(path.to_path_buf())
            .or_insert_with(|| std::fs::read_to_string(path).unwrap_or_default())
    }
}

/// A call hierarchy answer as targets: functions of the project where its
/// items point into it, else the package outside they're in.
pub(super) fn targets(
    project: &Project,
    answer: &Value,
    side: &str,
    encoding: lsp::Encoding,
    texts: &mut Texts,
) -> Vec<Target> {
    let mut found: Vec<Target> = Vec::new();
    for call in answer.as_array().into_iter().flatten() {
        let item = &call[side];
        let target = item["uri"]
            .as_str()
            .and_then(lsp::uri_path)
            .and_then(|path| {
                let relative = canonical(&path)
                    .strip_prefix(canonical(&project.root))
                    .ok()?
                    .to_path_buf();
                let range =
                    serde_json::from_value::<lsp_types::Range>(item["selectionRange"].clone())
                        .ok()?;
                let offset = lsp::offset(texts.of(&path), range.start, encoding);
                project.function_at(&relative, offset).map(Target::Function)
            })
            .unwrap_or_else(|| Target::External(package_of(item)));
        if !found.contains(&target) {
            found.push(target);
        }
    }
    found
}

/// The package an item outside the project is in, as the server says it.
pub(super) fn package_of(item: &Value) -> String {
    let detail = item["detail"].as_str().unwrap_or("");
    if !detail.is_empty() {
        // jdtls: `org.springframework.http.ResponseEntity`.
        let package: Vec<&str> = detail
            .split(['.', ' '])
            .take_while(|part| part.starts_with(char::is_lowercase))
            .collect();
        if !package.is_empty() {
            return package.into_iter().take(3).collect::<Vec<_>>().join(".");
        }
    }
    let uri = item["uri"].as_str().unwrap_or("");
    // node_modules/<package>/…
    if let Some(at) = uri.find("node_modules/") {
        let rest = &uri[at + "node_modules/".len()..];
        let mut parts = rest.split('/');
        let first = parts.next().unwrap_or("");
        return if first.starts_with('@') {
            format!("{first}/{}", parts.next().unwrap_or(""))
        } else {
            first.to_string()
        };
    }
    "outside the project".into()
}
