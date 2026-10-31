//! The Map in place of the code: ⌘B (Show Map), the title bar's Map
//! button or a pinch-out in the code flies out from the function at the
//! cursor to its Plan, and zooming into a Plan's own function, or Esc,
//! lands back in the code. Anything that puts the cursor in the code
//! (another tab, Go to Line, a problem) shows the code again. Entering
//! the Map and each level it moves through are places for Go Back.
//!
//! The project is read on a background thread when the Map first shows
//! and read again, file by file, each time it shows; the language server,
//! where there is one, says what a Plan's function calls and is called
//! by. While debugging, the paused program's stack lights up on the Plan,
//! which follows the pause, and running functions glow as they're
//! entered.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use gpui_kit::*;
use jig_editor::EditorHandle;
use serde_json::{Value, json};

use super::definitions::with_timeout;
use super::mode::Mode;
use super::tabs::canonical;
use super::{ExplainLevel, IndexDomains, ShowMap, UpdateDomains, Workspace};
use crate::lsp;
use jig_map::{Action, FnId, Level, MapEvent, MapView, Project, Target, ruler};

/// How long the language server has to answer, and how many times it's
/// asked while it may still be indexing (jdtls takes minutes cold).
const SERVER_TIMEOUT: Duration = Duration::from_secs(15);
const SERVER_ATTEMPTS: usize = 6;
const SERVER_RETRY: Duration = Duration::from_secs(5);
/// A pinch in the code this far out shows the Map.
const PINCH_OUT: f32 = -0.12;
/// Where the ruler's needle is as the pinch shows the Map: halfway from
/// Code to Plan.
const PINCH_DEPTH: f32 = 0.5;

pub(super) struct MapPane {
    pub(super) view: Entity<MapView>,
    _events: Subscription,
}

impl Workspace {
    /// The Map, if it's showing.
    pub(super) fn map_showing(&self) -> Option<&Entity<MapView>> {
        self.map
            .as_ref()
            .filter(|_| self.mode == Mode::Map)
            .map(|pane| &pane.view)
    }

    /// ⌘B: the Map, flown out of the function at the cursor; pressed
    /// while it shows, back to the code.
    pub(super) fn show_map(&mut self, _: &ShowMap, window: &mut Window, cx: &mut Context<Self>) {
        if self.map_showing().is_some() {
            self.close_map(window, cx);
            return;
        }
        self.map_from_code(window, cx);
    }

    /// Out of the code to the Map: the Plan of the function at the
    /// cursor, else of its file; Go Back returns to the code.
    pub(super) fn map_from_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = (!self.home).then(|| self.document().path.clone()).flatten();
        let offset = self.editor().cursor(cx);
        self.remember_location(cx);
        self.leave_tab(cx);
        let Some(view) = self.open_map_pane(window, cx) else {
            return;
        };
        self.read_and(&view, cx, move |view, cx| match path {
            Some(path) => view.arrive_from_code(&path, offset, cx),
            None => view.go_to(Level::Site, cx),
        });
    }

    /// Show the Map where it was left; read the project for it.
    pub(super) fn open_map_pane(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<MapView>> {
        // The folder open in the sidebar, else the file's project.
        let file = (!self.home).then(|| self.document().path.clone()).flatten();
        let open = self.project_root(cx);
        // The whole project the file's in, even with only a folder of it
        // open in the sidebar.
        let root = match file {
            Some(file) => {
                let project = jig_map::root_of(&file);
                match open {
                    Some(open) if file.starts_with(&open) && !open.starts_with(&project) => {
                        Some(open)
                    }
                    _ => Some(project),
                }
            }
            None => open,
        };
        let Some(root) = root else {
            self.show_note(
                "Open the project's folder, or save the file, to see its Map.".into(),
                window,
                cx,
            );
            return None;
        };
        if self
            .map
            .as_ref()
            .is_some_and(|pane| pane.view.read(cx).root() != root)
        {
            self.map = None;
        }
        if self.map.is_none() {
            let view = cx.new(|cx| MapView::new(root, cx));
            let events = cx.subscribe_in(&view, window, |this, view, event, window, cx| {
                this.on_map_event(view, event, window, cx)
            });
            self.map = Some(MapPane {
                view,
                _events: events,
            });
        }
        let view = self.map.as_ref().expect("made above").view.clone();
        self.switch_mode(Mode::Map, cx);
        view.focus_handle(cx).focus(window, cx);
        self.sync_live_points(cx);
        self.refresh_map_debug(cx);
        self.sync_map_drawings(cx);
        cx.notify();
        Some(view)
    }

    /// View > Index Domains or Update Domains: the Map at Domain, and the
    /// agent asked.
    pub(super) fn index_domains(
        &mut self,
        _: &IndexDomains,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.map_action_at_domain(false, window, cx);
    }

    /// View > Explain This Level: the agent asked about the Map's level,
    /// the Map shown first if it isn't.
    pub(super) fn explain_level(
        &mut self,
        _: &ExplainLevel,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = self.map_showing().cloned() {
            return view.update(cx, |view, cx| view.explain_level(cx));
        }
        self.leave_tab(cx);
        let path = (!self.home).then(|| self.document().path.clone()).flatten();
        let offset = self.editor().cursor(cx);
        let Some(view) = self.open_map_pane(window, cx) else {
            return;
        };
        self.read_and(&view, cx, move |view, cx| {
            if let Some(path) = path {
                view.arrive_from_code(&path, offset, cx);
            }
            view.explain_level(cx);
        });
    }

    pub(super) fn update_domains(
        &mut self,
        _: &UpdateDomains,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.map_action_at_domain(true, window, cx);
    }

    fn map_action_at_domain(&mut self, update: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_tab(cx);
        let Some(view) = self.open_map_pane(window, cx) else {
            return;
        };
        self.read_and(&view, cx, move |view, cx| {
            view.go_to(Level::Domain, cx);
            if update {
                view.update_domains(cx);
            } else {
                view.start_action(Action::Index, cx);
            }
        });
    }

    /// Back to the code, remembering the Map's place for Go Back.
    pub(super) fn close_map(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.map_showing().is_some() {
            self.remember_location(cx);
        }
        self.leave_map(window, cx);
    }

    /// Back to the code, the Map's view kept as it's arranged.
    pub(super) fn leave_map(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hide_map(cx) {
            self.focus_main(window, cx);
        }
    }

    /// The Map stops showing, the code in its place; whether it was.
    fn hide_map(&mut self, cx: &mut Context<Self>) -> bool {
        if self.mode != Mode::Map {
            return false;
        }
        self.switch_mode(Mode::Code, cx);
        true
    }

    /// The code got focus some other way (Go to Line, a problem, another
    /// tab): it shows, in place of the Planner or the Map.
    pub(super) fn code_focused(&mut self, cx: &mut Context<Self>) {
        self.switch_mode(Mode::Code, cx);
    }

    /// Go Back or Forward to `level` of the Map.
    pub(super) fn map_at(&mut self, level: Level, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_tab(cx);
        let Some(view) = self.open_map_pane(window, cx) else {
            return;
        };
        self.read_and(&view, cx, move |view, cx| view.go_to(level, cx));
    }

    /// Read the project again (it may have changed since: only files that
    /// did are read), then run `then` on the Map.
    pub(super) fn read_and(
        &mut self,
        view: &Entity<MapView>,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut MapView, &mut Context<MapView>) + 'static,
    ) {
        let root = view.read(cx).root().to_path_buf();
        let before = view.read(cx).project().cloned();
        let view = view.downgrade();
        let read =
            cx.background_spawn(async move { Project::read_again(&root, before.as_deref()) });
        cx.spawn(async move |_, cx| {
            let project = read.await;
            view.update(cx, |view, cx| {
                view.set_project(Arc::new(project), cx);
                then(view, cx);
            })
            .ok();
        })
        .detach();
    }

    fn on_map_event(
        &mut self,
        view: &Entity<MapView>,
        event: &MapEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            MapEvent::Moved(level) => self.remember_map(level.clone()),
            MapEvent::OpenCode(path, at) => {
                let (path, at) = (path.clone(), *at);
                self.close_map(window, cx);
                self.open_file(&path, window, cx);
                if self.showing(&path) {
                    self.editor().select(at..at, cx);
                    self.focus_editor(window, cx);
                }
            }
            MapEvent::OpenFile(path) => {
                let path = path.clone();
                self.close_map(window, cx);
                self.open_file(&path, window, cx);
            }
            MapEvent::AskServer(id) => self.map_server_calls(view, *id, window, cx),
            MapEvent::LiveLines => self.sync_live_points(cx),
            MapEvent::Close => self.close_map(window, cx),
            MapEvent::Ask(action, instruction) => {
                let root = view.read(cx).root().to_path_buf();
                self.start_map_action(action.clone(), instruction.clone(), root, window, cx);
            }
            MapEvent::Explain(instruction) => self.start_map_conversation(
                Some("Explain this level".into()),
                instruction.clone(),
                window,
                cx,
            ),
            MapEvent::Settled(at, kept) => self.drawing_settled(*at, *kept, cx),
            MapEvent::Proposal(at, choice) => self.proposal_chosen(*at, *choice, window, cx),
            MapEvent::Converse(name, instruction) => {
                self.start_sketch_conversation(name.clone(), instruction.clone(), window, cx)
            }
            MapEvent::Implement(task, sketch) => {
                let (task, sketch) = (task.clone(), sketch.clone());
                self.implement_sketch_task(&task, &sketch, window, cx);
            }
            MapEvent::OpenTask(id) => {
                let id = id.clone();
                self.show_task(&id, window, cx);
            }
        }
    }

    /// A pinch in the code: far enough out, the Map.
    pub(super) fn pinch_code(
        &mut self,
        event: &PinchEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.phase == TouchPhase::Ended {
            self.pinched = 0.;
            cx.notify();
            return;
        }
        self.pinched = (self.pinched + event.delta).min(0.);
        if self.pinched <= PINCH_OUT && !self.home && self.map_showing().is_none() {
            self.pinched = 0.;
            self.map_from_code(window, cx);
            if let Some(view) = self.map_showing() {
                view.read(cx).ruler_from(PINCH_DEPTH);
            }
        }
        cx.notify();
    }

    /// The Map's ruler over the code while a pinch is on its way out: its
    /// needle going from Code towards Plan, halfway there as the Map shows.
    pub(super) fn pinch_ruler(&self, cx: &App) -> Option<AnyElement> {
        if self.pinched >= 0. || self.home {
            return None;
        }
        let depth = self.pinched / PINCH_OUT * PINCH_DEPTH;
        Some(ruler(
            depth,
            "Code",
            jig_kit::theme::editor_surface(cx),
            None,
            cx,
        ))
    }

    /// Ask the language server what function `id` calls and is called by.
    fn map_server_calls(
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
    pub(super) fn refresh_map_debug(&self, cx: &mut App) {
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

    /// The debugger paused in `path`, the Map showing: it stays, on the
    /// Plan of where it paused.
    pub(super) fn keep_map(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.map.as_ref().map(|pane| pane.view.clone()) {
            self.switch_mode(Mode::Map, cx);
            view.focus_handle(cx).focus(window, cx);
            self.refresh_map_debug(cx);
            cx.notify();
        }
    }

    /// Focus what the mode shows: the Map, the Planner, or else the
    /// current tab's code.
    pub(super) fn focus_code(&self, window: &mut Window, cx: &mut Context<Self>) {
        if self.mode == Mode::Agent {
            return self.focus_agent_mode(window, cx);
        }
        if self.mode == Mode::Planner {
            return self.focus_planner_mode(window, cx);
        }
        match self.map_showing() {
            Some(view) => view.focus_handle(cx).focus(window, cx),
            None => self.editor().focus(window, cx),
        }
    }
}

/// Files read once each while the server's answer is mapped.
#[derive(Default)]
struct Texts(HashMap<PathBuf, String>);

impl Texts {
    fn of(&mut self, path: &Path) -> &str {
        self.0
            .entry(path.to_path_buf())
            .or_insert_with(|| std::fs::read_to_string(path).unwrap_or_default())
    }
}

/// A call hierarchy answer as targets: functions of the project where its
/// items point into it, else the package outside they're in.
fn targets(
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
fn package_of(item: &Value) -> String {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Texts, package_of, targets};
    use crate::lsp;
    use jig_map::{Project, Target};

    #[test]
    fn a_call_hierarchy_answer_maps_onto_the_project() {
        let dir = jig_map::fixture_project();
        let project = Project::read(dir.path());
        let users = dir
            .path()
            .join("server/src/org/demo/users/UserService.java");
        let answer = json!([
            {"to": {
                "name": "find", "kind": 6,
                "uri": lsp::file_uri(&users),
                "range": {"start": {"line": 3, "character": 4}, "end": {"line": 5, "character": 5}},
                "selectionRange": {"start": {"line": 3, "character": 16}, "end": {"line": 3, "character": 20}}
            }, "fromRanges": []},
            {"to": {
                "name": "info", "kind": 6, "detail": "org.slf4j.Logger",
                "uri": "jdt://contents/slf4j-api.jar/org.slf4j/Logger.class",
                "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                "selectionRange": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}
            }, "fromRanges": []}
        ]);
        let found = targets(
            &project,
            &answer,
            "to",
            lsp::Encoding::Utf16,
            &mut Texts::default(),
        );
        let find = (0..project.functions.len())
            .find(|&id| project.functions[id].title() == "UserService.find")
            .unwrap();
        assert_eq!(
            found,
            [Target::Function(find), Target::External("org.slf4j".into())]
        );
    }

    #[test]
    fn packages_outside_are_named_from_the_detail_or_node_modules() {
        assert_eq!(
            package_of(&json!({"detail": "org.springframework.http.ResponseEntity"})),
            "org.springframework.http"
        );
        assert_eq!(
            package_of(&json!({"uri": "file:///p/node_modules/@scope/lib/index.d.ts"})),
            "@scope/lib"
        );
    }
}
