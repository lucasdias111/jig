//! Run configurations: choosing one (⌃⌥R), running it (⌃R), stopping it
//! (⌘F2), and the panel under the editor that shows its output (⌘J).
//!
//! One configuration runs at a time; running another, or the same again,
//! stops the one before. Unsaved files are saved first, as IntelliJ does,
//! so what runs is what's on screen.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures::channel::mpsc::{TryRecvError, UnboundedReceiver};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_editor::EditorHandle as _;

use super::{
    ChooseRunConfiguration, EditRunConfigurations, RunSelected, StopRun, TITLE_BAR_HEIGHT,
    ToggleRunPanel, Workspace,
};
use jig_run::run_configs::{self, RunConfig};
use jig_run::run_output::{Link, OutputLine, Process, RunEvent};
use jig_run::run_picker::PLAY;

mod picker;
mod session;
mod view;

pub(super) mod configurations;
pub(super) mod output;

use configurations::{RunConfigurations, RunConfigurationsEvent};
use output::{RunOutput, RunOutputEvent};

const HEADER_HEIGHT: f32 = 30.;
const MIN_PANEL_HEIGHT: f32 = 90.;
/// Room the panel leaves the editor above it, at least.
const MIN_EDITOR_HEIGHT: f32 = 160.;
/// Events taken from the process per update, so a flood still repaints.
const MAX_BATCH: usize = 2_000;
/// How often new output is looked for.
const POLL_INTERVAL: Duration = Duration::from_millis(30);

const STOP: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="black"><rect x="5.5" y="5.5" width="13" height="13" rx="2"/></svg>"#;
const RERUN: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M20 11a8 8 0 1 0-2.3 5.7"/><path d="M20 4v7h-7"/></svg>"#;
const CLOSE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>"#;
const CHEVRON: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="black" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round"><path d="m6 9 6 6 6-6"/></svg>"#;

pub(super) struct RunState {
    pub(super) session: Option<RunSession>,
    pub(super) panel_open: bool,
    panel_height: Pixels,
    /// Set while the panel's top edge is being dragged.
    resizing: bool,
    next_id: u64,
}

impl RunState {
    pub(super) fn new() -> Self {
        Self {
            session: None,
            panel_open: false,
            panel_height: px(220.),
            resizing: false,
            next_id: 0,
        }
    }
}

/// One run of a configuration. What it printed is in the window's
/// [`RunOutput`]. Debugging is a run with a debugger attached.
pub(super) struct RunSession {
    id: u64,
    pub(super) config: RunConfig,
    /// The program, or for a debug session, its build.
    pub(super) process: Option<Process>,
    events: UnboundedReceiver<RunEvent>,
    /// How it ended, once it has.
    pub(super) ending: Option<String>,
    pub(super) started: Instant,
    pub(super) debug: Option<super::debug::DebugSession>,
    _poll: Task<()>,
}

impl RunSession {
    fn is_running(&self) -> bool {
        self.ending.is_none()
            && (self.process.as_ref().is_some_and(Process::is_running)
                || self.debug.as_ref().is_some_and(|debug| debug.is_live()))
    }
}

impl Workspace {
    /// The project configurations belong to: the sidebar's, or the open
    /// file's.
    pub(super) fn run_root(&self, cx: &App) -> Option<PathBuf> {
        self.project_root(cx).or_else(|| {
            self.document()
                .path
                .as_deref()
                .map(jig_kit::project::root_for)
        })
    }
}
