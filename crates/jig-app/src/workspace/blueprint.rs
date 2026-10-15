//! The blueprint view in place of a tab's code (⌘B): the function at the
//! cursor as blocks that flow, and a click on one goes back to the code
//! with its lines selected. Anything that puts the cursor in the code
//! (Go to Line, a problem, Go Back) shows the code again too.
//!
//! The functions calling the one shown come from the language server's
//! call hierarchy, or without one, from this file; a caller, or a call's
//! button, opens that function's blueprint, and Go Back returns.
//!
//! While debugging, a block's dot sets a breakpoint on its line, the block
//! the debugger is paused in is marked and followed, and the way it went
//! this session is lit, with how many times it paused in each block.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_editor::EditorHandle;
use serde_json::{Value, json};

use super::breakpoints::{line_of, line_range};
use super::definitions::with_timeout;
use super::tabs::canonical;
use super::{ToggleBlueprint, Workspace};
use crate::blueprint::debug::DebugMarks;
use crate::blueprint::{self, Caller, Function, Lang, view::BlueprintEvent, view::BlueprintView};
use crate::lsp;

/// How long the language server has to name a function's callers before
/// this file's are shown instead.
const CALLERS_TIMEOUT: Duration = Duration::from_secs(10);
/// A server still indexing knows no function yet; it's asked again this
/// many times, this far apart.
const CALLERS_ATTEMPTS: usize = 4;
const CALLERS_RETRY: Duration = Duration::from_secs(2);
/// More pauses than anyone steps through in one session.
const MAX_TRAIL: usize = 2000;

pub(super) struct OpenBlueprint {
    pub(super) view: Entity<BlueprintView>,
    _events: Subscription,
    /// The code getting focus, which shows it again.
    _code_focused: Subscription,
}

impl Workspace {
    pub(super) fn toggle_blueprint(
        &mut self,
        _: &ToggleBlueprint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.home {
            return;
        }
        if self.tab().blueprint.is_some() {
            self.close_blueprint(window, cx);
            return;
        }
        if !blueprint::reads(&self.editor().language(cx)) {
            self.show_note(
                "The blueprint view reads Rust, JavaScript and TypeScript for now.".into(),
                window,
                cx,
            );
            return;
        }
        self.open_blueprint(window, cx);
    }

    /// Show the current tab's blueprint, of the function at the cursor.
    fn open_blueprint(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_tab(cx);
        let source = self.editor().text(cx);
        let cursor = self.editor().cursor(cx);
        let Some(lang) = Lang::of(&self.editor().language(cx)) else {
            return;
        };
        let view = cx.new(|cx| BlueprintView::new(source, cursor, lang, cx));
        let events = cx.subscribe_in(&view, window, |this, view, event, window, cx| match event {
            BlueprintEvent::Open(range) => this.open_from_blueprint(range.clone(), window, cx),
            BlueprintEvent::Close => this.close_blueprint(window, cx),
            BlueprintEvent::Shown(function) => this.find_callers(view, function, window, cx),
            BlueprintEvent::OpenCaller(caller) => {
                let path = caller.path.clone().or_else(|| this.document().path.clone());
                if let Some(path) = path {
                    this.blueprint_in(&path, caller.at, window, cx);
                }
            }
            BlueprintEvent::StepInto(callee) => this.open_callee(*callee, window, cx),
            BlueprintEvent::ToggleBreakpoint(at) => {
                let line = line_of(&this.editor().text(cx), *at);
                let id = this.tab().id();
                this.toggle_breakpoint_line(id, line, window, cx);
            }
            BlueprintEvent::LiveLines => this.sync_live_points(cx),
        });
        let code = self.editor().state().focus_handle(cx);
        let tab = self.tab().id();
        let code_focused = cx.on_focus(&code, window, move |this, _, cx| {
            if let Some(tab) = this.tabs.iter_mut().find(|open| open.id() == tab) {
                tab.blueprint = None;
                this.sync_live_points(cx);
                cx.notify();
            }
        });
        view.focus_handle(cx).focus(window, cx);
        let function = view.read(cx).current_function().cloned();
        self.tab_mut().blueprint = Some(OpenBlueprint {
            view: view.clone(),
            _events: events,
            _code_focused: code_focused,
        });
        // Drawn before anything heard it, so its callers are asked for here.
        if let Some(function) = function {
            self.find_callers(&view, &function, window, cx);
        }
        self.refresh_blueprint_debug(cx);
        cx.notify();
    }

    /// Back to the code, as it was.
    pub(super) fn close_blueprint(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.tab_mut().blueprint.take().is_some() {
            self.sync_live_points(cx);
            self.editor().focus(window, cx);
            cx.notify();
        }
    }

    /// A block was clicked: its code, selected, with Go Back to return.
    fn open_from_blueprint(
        &mut self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remember_location(cx);
        self.close_blueprint(window, cx);
        self.editor().select(range, cx);
    }

    /// The current tab's blueprint at `offset`: the function there, the
    /// block there outlined. Opens the view if it isn't showing.
    pub(super) fn blueprint_at(
        &mut self,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !blueprint::reads(&self.editor().language(cx)) {
            self.editor().focus(window, cx);
            return;
        }
        self.editor().select(offset..offset, cx);
        match self.blueprint().cloned() {
            Some(view) => {
                let source = self.editor().text(cx);
                view.update(cx, |view, cx| view.refresh(source, offset, cx));
                view.focus_handle(cx).focus(window, cx);
            }
            None => self.open_blueprint(window, cx),
        }
        if let Some(view) = self.blueprint().cloned() {
            view.update(cx, |view, cx| view.show_at(offset, cx));
        }
    }

    /// Jump to the blueprint of `path` at `offset`, with Go Back to return.
    fn blueprint_in(
        &mut self,
        path: &Path,
        offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.remember_location(cx);
        self.open_file(path, window, cx);
        if self.showing(path) {
            self.blueprint_at(offset, window, cx);
        }
    }

    /// A call's button: the blueprint of the function it calls, found as
    /// Go to Definition finds it.
    fn open_callee(&mut self, callee: usize, window: &mut Window, cx: &mut Context<Self>) {
        let found = self.declaration_of(callee, window, cx);
        cx.spawn_in(window, async move |this, cx| {
            let found = found.await;
            this.update_in(cx, |this, window, cx| match found {
                Some((path, at)) => this.blueprint_in(&path, at, window, cx),
                None => this.show_note("Couldn't find the function it calls.".into(), window, cx),
            })
            .ok();
        })
        .detach();
    }

    /// Find what calls `function`, shown in `view`, and hand it the list.
    fn find_callers(
        &mut self,
        view: &Entity<BlueprintView>,
        function: &Function,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.tabs.iter().position(|tab| {
            tab.blueprint
                .as_ref()
                .is_some_and(|open| &open.view == view)
        }) else {
            return;
        };
        let tab = &self.tabs[ix];
        let source = tab.editor.text(cx);
        let path = tab.document.path.clone();
        let lang = view.read(cx).lang();
        let here: Vec<Caller> = blueprint::callers_in(&source, lang, function)
            .into_iter()
            .map(|(name, at)| Caller {
                place: place(path.as_deref(), &source, at),
                name,
                path: path.clone(),
                at,
            })
            .collect();
        let view = view.downgrade();
        let name = function.name.clone();
        let Some(lsp) = tab.lsp.as_ref() else {
            view.update(cx, |view, cx| view.set_callers(&name, here, cx))
                .ok();
            return;
        };
        let client = lsp.client.clone();
        let encoding = client.encoding();
        let position = lsp::position(&source, function.name_at, encoding);
        let params = lsp::text_document_position(&lsp.uri, position);
        cx.spawn_in(window, async move |this, cx| {
            let mut item = None;
            for attempt in 0..CALLERS_ATTEMPTS {
                if attempt > 0 {
                    cx.background_executor().timer(CALLERS_RETRY).await;
                }
                let prepare = client.request("textDocument/prepareCallHierarchy", params.clone());
                item = with_timeout(prepare, CALLERS_TIMEOUT, cx)
                    .await
                    .and_then(|items| items.as_array()?.first().cloned());
                if item.is_some() {
                    break;
                }
            }
            let calls = match item {
                Some(item) => {
                    let request =
                        client.request("callHierarchy/incomingCalls", json!({"item": item}));
                    with_timeout(request, CALLERS_TIMEOUT, cx).await
                }
                None => None,
            };
            let callers = match calls.as_ref().and_then(Value::as_array) {
                // Empty, a server may still be indexing: this file's
                // stand, if any.
                Some(calls) if !calls.is_empty() || here.is_empty() => {
                    let calls = incoming_calls(calls);
                    this.read_with(cx, |this, cx| callers_from(this, calls, encoding, cx))
                        .unwrap_or_default()
                }
                // No answer: this file's, by name.
                _ => here,
            };
            view.update(cx, |view, cx| view.set_callers(&name, callers, cx))
                .ok();
        })
        .detach();
    }

    /// In the title bar, left of the Jig button: swaps a readable file's code
    /// for its blueprint and back, pressed while the blueprint shows.
    pub(super) fn render_blueprint_button(&self, cx: &Context<Self>) -> Option<AnyElement> {
        if self.home || !blueprint::reads(&self.editor().language(cx)) {
            return None;
        }
        let theme = cx.theme();
        let showing = self.blueprint().is_some();
        let accent = theme.primary;
        let shortcut = if cfg!(target_os = "macos") {
            "⌘B"
        } else {
            "Ctrl B"
        };
        Some(
            h_flex()
                .id("blueprint-toggle")
                .debug_selector(|| "blueprint-toggle".into())
                .flex_none()
                .ml_2()
                .h(px(26.))
                .px_2()
                .gap_1p5()
                .rounded(px(6.))
                .text_size(px(12.5))
                .text_color(theme.foreground.opacity(0.85))
                .when(showing, |this| {
                    this.bg(accent.opacity(0.16)).text_color(accent)
                })
                .when(!showing, |this| {
                    this.hover(|style| style.bg(theme.foreground.opacity(0.08)))
                })
                .child(
                    Icon::new(IconName::LayoutDashboard)
                        .size(px(13.))
                        .flex_none()
                        .text_color(accent),
                )
                .child(div().flex_none().whitespace_nowrap().child(if showing {
                    "Code"
                } else {
                    "Blueprint"
                }))
                .child(
                    div()
                        .flex_none()
                        .text_size(px(11.))
                        .text_color(theme.muted_foreground)
                        .child(shortcut),
                )
                .tooltip(move |window, cx| {
                    let text = if showing {
                        "Back to the code"
                    } else {
                        "The function at the cursor, as blocks"
                    };
                    Tooltip::new(text).build(window, cx)
                })
                // Otherwise the title bar takes the press as a window drag.
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, window, cx| {
                    this.toggle_blueprint(&ToggleBlueprint, window, cx)
                }))
                .into_any_element(),
        )
    }

    /// The blueprint view showing in the current tab, if any.
    pub(super) fn blueprint(&self) -> Option<&Entity<BlueprintView>> {
        self.tab().blueprint.as_ref().map(|open| &open.view)
    }

    /// Focus the current tab's blueprint view if it's showing, else its
    /// code.
    pub(super) fn focus_code(&self, window: &mut Window, cx: &mut Context<Self>) {
        match self.blueprint() {
            Some(view) => view.focus_handle(cx).focus(window, cx),
            None => self.editor().focus(window, cx),
        }
    }

    /// Tab `ix`'s code changed, from outside Jig or by the agent: draw it
    /// again.
    pub(super) fn blueprint_changed(&mut self, ix: usize, cx: &mut Context<Self>) {
        let tab = &self.tabs[ix];
        let Some(open) = tab.blueprint.as_ref() else {
            return;
        };
        let source = tab.editor.text(cx);
        let cursor = tab.editor.cursor(cx);
        open.view
            .update(cx, |view, cx| view.refresh(source, cursor, cx));
        // Breakpoints move with the lines.
        self.refresh_blueprint_debug(cx);
    }

    /// The debugger paused at `line` (0-based) of `path`: on the trail.
    pub(super) fn note_pause(&mut self, path: &Path, line: u32) {
        let place = (canonical(path), line);
        if self.debug_trail.last() != Some(&place) {
            self.debug_trail.push(place);
        }
        if self.debug_trail.len() > MAX_TRAIL {
            self.debug_trail.remove(0);
        }
    }

    /// Hand each blueprint showing the debugger's marks in its file.
    pub(super) fn refresh_blueprint_debug(&self, cx: &mut App) {
        for (ix, tab) in self.tabs.iter().enumerate() {
            let Some(open) = tab.blueprint.as_ref() else {
                continue;
            };
            let text = tab.editor.text(cx);
            let path = tab.document.path.as_deref().map(canonical);
            let code_on =
                |line: u32| line_range(&text, line).map(|range| first_code(&text, range.start));
            let marks = DebugMarks {
                enabled: self.takes_breakpoints_in(ix),
                breakpoints: self
                    .breakpoint_lines(ix, cx)
                    .into_iter()
                    .filter_map(code_on)
                    .collect(),
                paused: self
                    .execution_offset(ix, cx)
                    .map(|at| first_code(&text, at)),
                trail: self
                    .debug_trail
                    .iter()
                    .filter(|(file, _)| Some(file) == path.as_ref())
                    .filter_map(|&(_, line)| code_on(line))
                    .collect(),
            };
            open.view.update(cx, |view, cx| view.set_debug(marks, cx));
        }
    }
}

/// A call the server reported: the file, the caller's name as it gives it,
/// and where the call is.
struct IncomingCall {
    path: PathBuf,
    name: String,
    at: lsp_types::Position,
}

/// `callHierarchy/incomingCalls`' answer, one per call site.
fn incoming_calls(calls: &[Value]) -> Vec<IncomingCall> {
    calls
        .iter()
        .filter_map(|call| {
            let from = call.get("from")?;
            let path = lsp::uri_path(from.get("uri")?.as_str()?)?;
            let name = from.get("name")?.as_str()?.to_string();
            let at = call
                .get("fromRanges")
                .and_then(|ranges| ranges.get(0))
                .or_else(|| from.get("selectionRange"))
                .and_then(|range| serde_json::from_value::<lsp_types::Range>(range.clone()).ok())?
                .start;
            Some(IncomingCall { path, name, at })
        })
        .collect()
}

/// The server's calls as callers, named as the blueprint names functions
/// (`Type::name`) where the file still has one around the call.
fn callers_from(
    workspace: &Workspace,
    calls: Vec<IncomingCall>,
    encoding: lsp::Encoding,
    cx: &App,
) -> Vec<Caller> {
    let mut callers: Vec<Caller> = calls
        .into_iter()
        .filter_map(|call| {
            let text = workspace
                .buffer_text(&call.path, cx)
                .or_else(|| std::fs::read_to_string(&call.path).ok())?;
            let at = lsp::offset(&text, call.at, encoding);
            let language =
                crate::languages::language_for(&call.path, &workspace.settings.languages);
            let functions = Lang::of(language)
                .map(|lang| blueprint::functions(&text, lang))
                .unwrap_or_default();
            let name = functions
                .iter()
                .filter(|function| function.range.contains(&at))
                .min_by_key(|function| function.range.len())
                .map_or(call.name, |function| function.name.clone());
            Some(Caller {
                place: place(Some(&call.path), &text, at),
                name,
                path: Some(call.path),
                at,
            })
        })
        .collect();
    callers.sort_by(|a, b| (&a.path, a.at).cmp(&(&b.path, b.at)));
    callers
}

/// The first code on the line from `start`, past its indentation, which
/// is where its block starts.
fn first_code(text: &str, start: usize) -> usize {
    let line = &text[start..];
    let indent = line.len() - line.trim_start_matches([' ', '\t']).len();
    start + indent
}

/// `workspace.rs:412`: the file's name and the line `at` is on.
fn place(path: Option<&Path>, text: &str, at: usize) -> String {
    let line = text[..at.min(text.len())].matches('\n').count() + 1;
    let file = path
        .and_then(Path::file_name)
        .map_or("Untitled".into(), |name| name.to_string_lossy());
    format!("{file}:{line}")
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::Value;

    use super::{incoming_calls, place};

    #[test]
    fn reads_incoming_calls() {
        let answer: Value = serde_json::from_str(
            r#"[
                {
                    "from": {
                        "name": "save_to",
                        "kind": 6,
                        "uri": "file:///project/src/workspace.rs",
                        "range": {"start": {"line": 9, "character": 0}, "end": {"line": 20, "character": 1}},
                        "selectionRange": {"start": {"line": 9, "character": 7}, "end": {"line": 9, "character": 14}}
                    },
                    "fromRanges": [{"start": {"line": 11, "character": 8}, "end": {"line": 11, "character": 12}}]
                },
                {"from": {"name": "broken"}}
            ]"#,
        )
        .unwrap();
        let calls = incoming_calls(answer.as_array().unwrap());
        assert_eq!(calls.len(), 1, "a call without a file is skipped");
        assert_eq!(calls[0].name, "save_to");
        assert_eq!(calls[0].path, Path::new("/project/src/workspace.rs"));
        assert_eq!(calls[0].at, lsp_types::Position::new(11, 8));
    }

    #[test]
    fn places_name_the_file_and_line() {
        let text = "a\nb\nc";
        assert_eq!(place(Some(Path::new("/x/lib.rs")), text, 4), "lib.rs:3");
        assert_eq!(place(None, text, 0), "Untitled:1");
    }
}
