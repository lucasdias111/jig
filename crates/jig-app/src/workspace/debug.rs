//! Debugging a run configuration (⌃D) with the debugger for its language
//! (`lldb-dap`, or `js-debug` on Node): build it, start it under the
//! debugger with the breakpoints set, and when it pauses show
//! where, the call stack and the variables, then resume (F9) or step (F8,
//! F7, ⇧F8).
//!
//! A debug session is a run session with a [`DebugSession`] attached: the
//! build's output and then the program's go to the same output panel,
//! which gains a Debugger view beside its Console.

use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::path::PathBuf;

use futures::channel::mpsc::{TryRecvError, UnboundedReceiver};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use serde_json::{Value, json};

use super::{DebugSelected, EditDebuggers, Resume, StepInto, StepOut, StepOver, Workspace};
use jig_debug::dap::{self, Message};
use jig_debug::debug_target::{self, Build, DebugPlan};
use jig_debuggers::{self as debuggers, Debugger, Vars};
use jig_run::run_configs::RunConfig;
use jig_run::run_output::{OutputLine, Stream, Style};

mod adapter;
mod controls;
mod messages;
mod panel;
mod starting;
use adapter::{faked, initialize_arguments, start_adapter, start_lsp_adapter};
#[cfg(test)]
use panel::variable_rows;

const ROW_HEIGHT: f32 = 20.;
/// Messages taken from the adapter per update.
const MAX_BATCH: usize = 500;
/// Frames asked for when the program pauses.
const MAX_FRAMES: u64 = 200;

pub(super) const BUG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 7.5a3 3 0 0 1 6 0"/><rect x="7" y="7.5" width="10" height="12.5" rx="5"/><path d="M12 11v9M3 13h4M17 13h4M4.5 7.5 7 9.5M19.5 7.5 17 9.5M4.5 19 7 17M19.5 19 17 17"/></svg>"#;
const RESUME: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="black" stroke="black" stroke-width="1.5" stroke-linejoin="round"><rect x="4" y="5" width="3" height="14" rx="1"/><path d="M10.5 5v14l10-7z"/></svg>"#;
const STEP_OVER: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 13a8 8 0 0 1 15-4"/><path d="M19.5 4v5.5H14"/><circle cx="12" cy="19" r="1.8" fill="black"/></svg>"#;
const STEP_INTO: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3v11"/><path d="m7 9.5 5 5 5-5"/><circle cx="12" cy="20" r="1.8" fill="black"/></svg>"#;
const STEP_OUT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 15V4"/><path d="m7 9 5-5 5 5"/><circle cx="12" cy="20" r="1.8" fill="black"/></svg>"#;
const CHEVRON_RIGHT: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m9 18 6-6-6-6"/></svg>"#;
const CHEVRON_DOWN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Phase {
    Building,
    Starting,
    Running,
    Paused,
    Ended,
}

/// Which half of the panel shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum View {
    Debugger,
    Console,
}

/// What a request was for, so its response can be acted on.
#[derive(Clone, Copy, Debug)]
enum Pending {
    Initialize,
    Launch,
    StackTrace,
    Scopes(i64),
    Variables(i64),
    /// Anything else; a failure is reported in the console.
    Command(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Frame {
    id: i64,
    pub(super) name: String,
    pub(super) path: Option<PathBuf>,
    /// 1-based.
    pub(super) line: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Variable {
    pub(super) name: String,
    pub(super) value: String,
    kind: Option<String>,
    /// Non-zero when it has children to ask for.
    reference: i64,
}

pub(super) struct DebugSession {
    plan: DebugPlan,
    /// The project, for `${root}`.
    root: PathBuf,
    /// The file open when debugging started, for `${file}`.
    file: Option<PathBuf>,
    /// Values a language server worked out for the launch, such as Java's
    /// main class; filled in before its adapter's port is known.
    lsp_values: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    pub(super) phase: Phase,
    pub(super) view: View,
    /// What to debug, once known: given, or reported by the build.
    executable: Option<PathBuf>,
    /// Connections to the adapter, by id: the session, then any child
    /// sessions it starts, as js-debug does for the program itself.
    clients: Vec<dap::Client>,
    /// The connection the program's pauses come on, which requests about
    /// the paused program go to.
    active: usize,
    /// The adapter, when it runs as a server.
    server: Option<dap::Server>,
    inbox: Option<dap::Inbox>,
    messages: Option<UnboundedReceiver<(usize, Message)>>,
    pending: HashMap<(usize, i64), Pending>,
    /// What each child session is to launch, once it's initialized.
    child_launches: HashMap<usize, Value>,
    /// Breakpoints are set and `configurationDone` sent.
    configured: bool,
    thread_id: Option<i64>,
    stop_reason: Option<String>,
    pub(super) frames: Vec<Frame>,
    selected_frame: usize,
    /// The selected frame's scopes, as variables with children.
    scopes: Vec<Variable>,
    children: HashMap<i64, Vec<Variable>>,
    expanded: HashSet<i64>,
    /// Program output not yet ended by a newline, which stream it's on,
    /// and its colour so far.
    partial: String,
    partial_stream: Stream,
    style: Style,
    frames_scroll: UniformListScrollHandle,
    variables_scroll: UniformListScrollHandle,
}

/// Starts a stand-in for the adapter in tests.
#[cfg(test)]
pub(super) type FakeAdapter = Box<dyn Fn(dap::Inbox) -> dap::Client>;

#[cfg(test)]
thread_local! {
    pub(super) static FAKE_ADAPTER: std::cell::RefCell<Option<FakeAdapter>> =
        const { std::cell::RefCell::new(None) };
}

impl Workspace {
    pub(super) fn debug_mut(&mut self) -> Option<&mut DebugSession> {
        self.runs.session.as_mut()?.debug.as_mut()
    }

    pub(super) fn debug(&self) -> Option<&DebugSession> {
        self.runs.session.as_ref()?.debug.as_ref()
    }

    /// The variables tree as shown, scopes and expanded variables flattened.
    #[cfg(test)]
    pub(super) fn debug_variable_rows(&self) -> Vec<(usize, String, String)> {
        self.debug()
            .map(|debug| {
                variable_rows(debug)
                    .into_iter()
                    .map(|row| (row.depth, row.variable.name, row.variable.value))
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn debug_phase(&self) -> Option<Phase> {
        self.debug().map(|debug| debug.phase)
    }

    #[cfg(test)]
    pub(super) fn debug_frames(&self) -> Vec<String> {
        self.debug()
            .map(|debug| debug.frames.iter().map(|f| f.name.clone()).collect())
            .unwrap_or_default()
    }

    #[cfg(test)]
    pub(super) fn expand_variable(&mut self, name: &str, cx: &mut Context<Self>) {
        let reference = self.debug().and_then(|debug| {
            variable_rows(debug)
                .into_iter()
                .find(|row| row.variable.name == name)
                .map(|row| row.variable.reference)
        });
        if let Some(reference) = reference {
            self.toggle_variable(reference, cx);
        }
    }
}

/// Breakpoints on `lines` (0-based), and logpoints, which print their
/// message there and carry on.
fn set_breakpoints_arguments(
    path: &std::path::Path,
    lines: &[u32],
    logpoints: &[(u32, String)],
) -> Value {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let breakpoints: Vec<Value> = lines
        .iter()
        .map(|line| json!({"line": line + 1}))
        .chain(
            logpoints
                .iter()
                .map(|(line, message)| json!({"line": line + 1, "logMessage": message})),
        )
        .collect();
    json!({
        "source": {"name": name, "path": path},
        "breakpoints": breakpoints,
        "sourceModified": false,
    })
}
