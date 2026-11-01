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
use jig_lsp::lsp;
use jig_map::{Action, FnId, Level, MapEvent, MapView, Project, Target, ruler};

mod actions;
mod calls;

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

#[cfg(test)]
mod tests;
