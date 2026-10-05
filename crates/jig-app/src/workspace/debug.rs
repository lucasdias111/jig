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
use crate::dap::{self, Message};
use crate::debug_target::{self, Build, DebugPlan};
use crate::debuggers::{self, Debugger, Vars};
use crate::run_configs::RunConfig;
use crate::run_output::{OutputLine, Stream, Style};

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
    path: Option<PathBuf>,
    /// 1-based.
    line: u32,
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

impl DebugSession {
    pub(super) fn is_live(&self) -> bool {
        self.phase != Phase::Ended
    }

    pub(super) fn status(&self) -> Option<String> {
        Some(match self.phase {
            Phase::Building => "Building…".into(),
            Phase::Starting => "Starting the debugger…".into(),
            Phase::Running => "Running".into(),
            Phase::Paused => match self.stop_reason.as_deref() {
                Some("breakpoint") | None => "Paused at a breakpoint".into(),
                Some("step") => "Paused".into(),
                Some("exception") => "Paused on a crash".into(),
                Some(reason) => format!("Paused ({reason})"),
            },
            Phase::Ended => return None,
        })
    }

    /// Send a request about the paused program.
    fn request(&mut self, command: &'static str, arguments: Value, pending: Pending) {
        self.request_to(self.active, command, arguments, pending);
    }

    fn request_to(&mut self, id: usize, command: &'static str, arguments: Value, pending: Pending) {
        if let Some(client) = self.clients.get(id) {
            let seq = client.request(command, arguments);
            self.pending.insert((id, seq), pending);
        }
    }

    /// Start a child session on the adapter server, as it asked.
    fn start_child(&mut self, configuration: Value) {
        let (Some(server), Some(inbox)) = (&self.server, &self.inbox) else {
            return;
        };
        let id = self.clients.len();
        self.clients
            .push(dap::Client::connect_tcp(id, server.port, inbox.clone()));
        self.child_launches.insert(id, configuration);
        self.active = id;
        let arguments = initialize_arguments(self.plan.debugger.adapter_id());
        self.request_to(id, "initialize", arguments, Pending::Initialize);
    }

    /// Paused state no longer holds once the program moves on.
    fn forget_pause(&mut self) {
        self.frames.clear();
        self.scopes.clear();
        self.children.clear();
        self.expanded.clear();
        self.selected_frame = 0;
        self.stop_reason = None;
    }

    /// Output as lines. The program's may come in pieces, so a trailing
    /// part waits for its newline; the debugger's own messages are whole.
    fn output_lines(&mut self, text: &str, stream: Stream) -> Vec<OutputLine> {
        let mut lines = Vec::new();
        if stream == Stream::Meta {
            return text
                .lines()
                .map(|line| OutputLine::meta(line.trim_end().to_string()))
                .collect();
        }
        if stream != self.partial_stream {
            lines.extend(self.flush_output());
            self.partial_stream = stream;
        }
        self.partial.push_str(text);
        while let Some(end) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=end).collect();
            lines.push(crate::run_output::parse_line(
                &line,
                stream,
                &mut self.style,
                &[],
            ));
        }
        lines
    }

    fn flush_output(&mut self) -> Option<OutputLine> {
        (!self.partial.is_empty()).then(|| {
            let line = std::mem::take(&mut self.partial);
            crate::run_output::parse_line(&line, self.partial_stream, &mut self.style, &[])
        })
    }
}

/// One row of the variables tree.
struct VariableRow {
    depth: usize,
    variable: Variable,
    expanded: bool,
}

/// Starts a stand-in for the adapter in tests.
#[cfg(test)]
pub(super) type FakeAdapter = Box<dyn Fn(dap::Inbox) -> dap::Client>;

#[cfg(test)]
thread_local! {
    pub(super) static FAKE_ADAPTER: std::cell::RefCell<Option<FakeAdapter>> =
        const { std::cell::RefCell::new(None) };
}

/// Start `debugger`'s adapter in `cwd` and connect to it, as connection 0.
fn start_adapter(
    debugger: &Debugger,
    cwd: &std::path::Path,
    inbox: dap::Inbox,
) -> anyhow::Result<(dap::Client, Option<dap::Server>)> {
    #[cfg(test)]
    if let Some(client) =
        FAKE_ADAPTER.with(|fake| fake.borrow().as_ref().map(|start| start(inbox.clone())))
    {
        return Ok((client, None));
    }
    let port = dap::free_port()?;
    let mut vars = Vars::new(cwd);
    vars.set("port", json!(port.to_string()));
    let adapter = debugger
        .adapter_command(&vars)
        .map_err(anyhow::Error::msg)?;
    if adapter.server {
        let server = dap::Server::start(&adapter, cwd, port)?;
        let client = dap::Client::connect_tcp(0, port, inbox);
        Ok((client, Some(server)))
    } else {
        Ok((dap::Client::start(0, &adapter, cwd, inbox)?, None))
    }
}

/// Start a debugger that lives in language server `server`: once it has
/// loaded the project and worked out what to launch, it says the port.
fn start_lsp_adapter(
    debugger: &Debugger,
    root: &std::path::Path,
    prefer: Vec<PathBuf>,
    values: std::sync::Arc<std::sync::Mutex<Vec<(String, Value)>>>,
    inbox: dap::Inbox,
    cx: &mut App,
) -> anyhow::Result<dap::Client> {
    let (server, start) = debugger
        .lsp()
        .ok_or_else(|| anyhow::anyhow!("{} doesn't live in a language server.", debugger.name))?;
    let language = debugger
        .languages
        .iter()
        .find(|language| crate::lsp::server_for(language).is_some_and(|s| s.name == server))
        .ok_or_else(|| anyhow::anyhow!("Jig doesn't run {server} for any of its languages."))?;
    let client = crate::lsp::client_for(&root.join("_"), language, cx)
        .ok_or_else(|| anyhow::anyhow!("{server} isn't installed."))?;
    let wanted = debuggers::lsp_bundles(server);
    let loaded = client.init_options()["bundles"].clone();
    if wanted.iter().any(|bundle| {
        !loaded
            .as_array()
            .is_some_and(|l| l.contains(&json!(bundle)))
    }) {
        anyhow::bail!(
            "{server} was started before its debugger plugin was installed. Quit and reopen \
             Jig to load it."
        );
    }
    let (port_tx, port_rx) = std::sync::mpsc::channel();
    let (server, start, root) = (server.to_string(), start.to_string(), root.to_path_buf());
    std::thread::spawn(move || {
        let started = crate::lsp_debug::start(&client, &server, &start, &root, &prefer);
        let _ = port_tx.send(started.map(|started| {
            *values.lock().unwrap() = started.values;
            started.port
        }));
    });
    Ok(dap::Client::connect_when(0, port_rx, inbox))
}

#[cfg(test)]
fn faked() -> bool {
    FAKE_ADAPTER.with(|fake| fake.borrow().is_some())
}

#[cfg(not(test))]
fn faked() -> bool {
    false
}

fn initialize_arguments(adapter_id: &str) -> Value {
    json!({
        "clientID": "jig",
        "clientName": "Jig",
        "adapterID": adapter_id,
        "linesStartAt1": true,
        "columnsStartAt1": true,
        "pathFormat": "path",
        "supportsVariableType": true,
        "supportsRunInTerminalRequest": false,
        "supportsStartDebuggingRequest": true,
    })
}

impl Workspace {
    /// ⌃D: debug the selected configuration, or choose one to.
    pub(super) fn debug_selected(
        &mut self,
        _: &DebugSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.runs.selected.clone() {
            Some(config) => self.start_debug(config, window, cx),
            None => self.open_run_picker_for(true, window, cx),
        }
    }

    pub(super) fn start_debug(
        &mut self,
        config: RunConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let plan = match debug_target::plan_for(&config, &debuggers::registry()) {
            Ok(plan) => plan,
            Err(why) => {
                self.show_error(&why, window, cx);
                return;
            }
        };
        let debugger = plan.debugger.clone();
        if !self.settings.debugging.is_enabled(&debugger.key) {
            self.show_error(
                &format!(
                    "Debugging {} is off. Turn it on in Settings, under Languages.",
                    debugger.name
                ),
                window,
                cx,
            );
            return;
        }
        if !faked()
            && let Some(missing) = debuggers::missing(&debugger, &config.cwd)
        {
            self.show_error(
                &format!(
                    "Jig can't debug {}: {missing} {}",
                    debugger.name,
                    debugger.help()
                ),
                window,
                cx,
            );
            return;
        }
        let executable = plan.program().map(std::path::Path::to_path_buf);
        let build = plan.build.command().map(|command| RunConfig {
            command,
            ..config.clone()
        });
        let json = matches!(plan.build, Build::Cargo(_));
        let root = self.run_root(cx).unwrap_or_else(|| config.cwd.clone());
        let file = (!self.home).then(|| self.document().path.clone()).flatten();
        let debug = DebugSession {
            plan,
            root,
            file,
            lsp_values: Default::default(),
            phase: Phase::Building,
            view: View::Console,
            executable,
            clients: Vec::new(),
            active: 0,
            server: None,
            inbox: None,
            messages: None,
            pending: HashMap::new(),
            child_launches: HashMap::new(),
            configured: false,
            thread_id: None,
            stop_reason: None,
            frames: Vec::new(),
            selected_frame: 0,
            scopes: Vec::new(),
            children: HashMap::new(),
            expanded: HashSet::new(),
            partial: String::new(),
            partial_stream: Stream::Stdout,
            style: Style::default(),
            frames_scroll: UniformListScrollHandle::new(),
            variables_scroll: UniformListScrollHandle::new(),
        };
        let building = build.is_some();
        self.start_session(config, build, json, Some(debug), window, cx);
        if !building {
            self.launch_debugger(window, cx);
        }
    }

    /// Open `~/.config/jig/debuggers.toml`, starting it from the template.
    pub(super) fn edit_debuggers(
        &mut self,
        _: &EditDebuggers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = debuggers::user_path() else {
            return;
        };
        if !path.exists() {
            let created = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, debuggers::USER_TEMPLATE));
            if let Err(error) = created {
                self.show_error(
                    &format!("Couldn't create {}: {error}", path.display()),
                    window,
                    cx,
                );
                return;
            }
        }
        self.open_file(&path, window, cx);
    }

    /// Cargo's JSON messages while building: the one naming the executable.
    pub(super) fn on_build_data(&mut self, json: &str) {
        if let Some(debug) = self.debug_mut()
            && let Some(executable) = debug_target::cargo_executable(json)
        {
            debug.executable = Some(executable);
        }
    }

    /// The build finished: debug what it built, or say why not. `stopped`
    /// when it was stopped with ⌘F2.
    pub(super) fn on_build_exit(
        &mut self,
        success: bool,
        stopped: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session) = self.runs.session.as_mut() else {
            return;
        };
        session.process = None;
        let Some(debug) = session.debug.as_mut() else {
            return;
        };
        let failure = if stopped {
            Some("Stopped")
        } else if !success {
            Some("Build failed")
        } else if matches!(debug.plan.build, Build::Cargo(_)) && debug.executable.is_none() {
            Some("The build made nothing to debug")
        } else {
            None
        };
        if let Some(failure) = failure {
            debug.phase = Phase::Ended;
            session.lines.push(OutputLine::meta(failure));
            session.ending = Some(failure.into());
            cx.notify();
            return;
        }
        self.launch_debugger(window, cx);
    }

    fn launch_debugger(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.runs.session.as_mut() else {
            return;
        };
        let Some(debug) = session.debug.as_mut() else {
            return;
        };
        let what = match (&debug.plan.build, &debug.executable) {
            (Build::Cargo(_), Some(executable)) => executable.display().to_string(),
            _ => debug.plan.describe(),
        };
        let (sender, receiver) = futures::channel::mpsc::unbounded();
        let debugger = debug.plan.debugger.clone();
        let started = match debugger.lsp().filter(|_| !faked()) {
            Some((server, _)) => {
                session.lines.push(OutputLine::meta(format!(
                    "Waiting for {server} to load the project…"
                )));
                let prefer: Vec<PathBuf> = debug
                    .plan
                    .program()
                    .map(std::path::Path::to_path_buf)
                    .into_iter()
                    .chain(debug.file.clone())
                    .collect();
                start_lsp_adapter(
                    &debugger,
                    &debug.root,
                    prefer,
                    debug.lsp_values.clone(),
                    sender.clone(),
                    cx,
                )
                .map(|client| (client, None))
            }
            None => start_adapter(&debugger, &session.config.cwd, sender.clone()),
        };
        match started {
            Ok((client, server)) => {
                debug.clients = vec![client];
                debug.server = server;
                debug.inbox = Some(sender);
                debug.messages = Some(receiver);
                debug.phase = Phase::Starting;
                session
                    .lines
                    .push(OutputLine::meta(format!("Debugging {what}")));
                let arguments = initialize_arguments(debugger.adapter_id());
                debug.request_to(0, "initialize", arguments, Pending::Initialize);
            }
            Err(error) => {
                let message = format!("{error:#}");
                debug.phase = Phase::Ended;
                session.lines.push(OutputLine::meta(message.clone()));
                session.ending = Some(message);
            }
        }
        cx.notify();
    }

    pub(super) fn debug_mut(&mut self) -> Option<&mut DebugSession> {
        self.runs.session.as_mut()?.debug.as_mut()
    }

    pub(super) fn debug(&self) -> Option<&DebugSession> {
        self.runs.session.as_ref()?.debug.as_ref()
    }

    /// What the adapter said since last time.
    pub(super) fn take_dap_messages(&mut self) -> Vec<(usize, Message)> {
        let mut batch = Vec::new();
        let Some(messages) = self.debug_mut().and_then(|debug| debug.messages.as_mut()) else {
            return batch;
        };
        while batch.len() < MAX_BATCH {
            match messages.try_recv() {
                Ok(message) => batch.push(message),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Closed) => {
                    batch.push((0, Message::Closed));
                    if let Some(debug) = self.debug_mut() {
                        debug.messages = None;
                    }
                    break;
                }
            }
        }
        batch
    }

    /// What connection `id` said.
    pub(super) fn on_dap_message(
        &mut self,
        id: usize,
        message: Message,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match message {
            Message::Response {
                request_seq,
                success,
                body,
                message,
            } => {
                let Some(pending) = self
                    .debug_mut()
                    .and_then(|debug| debug.pending.remove(&(id, request_seq)))
                else {
                    return;
                };
                let failure = message.unwrap_or_else(|| "it failed".into());
                match pending {
                    Pending::Initialize if success && id == 0 => self.send_launch(cx),
                    Pending::Initialize if success => {
                        if let Some(debug) = self.debug_mut()
                            && let Some(configuration) = debug.child_launches.remove(&id)
                        {
                            debug.request_to(id, "launch", configuration, Pending::Launch);
                        }
                    }
                    Pending::Initialize | Pending::Launch if !success && id == 0 => {
                        self.end_debugging(&format!("The debugger couldn't start: {failure}"), cx)
                    }
                    Pending::Initialize | Pending::Launch if !success => {
                        self.push_meta(format!("A child session couldn't start: {failure}"));
                    }
                    Pending::StackTrace if success => self.on_stack_trace(&body, window, cx),
                    Pending::Scopes(frame) if success => self.on_scopes(frame, &body),
                    Pending::Variables(reference) if success => {
                        if let Some(debug) = self.debug_mut() {
                            debug.children.insert(reference, parse_variables(&body));
                        }
                    }
                    Pending::Command(command) if !success => {
                        self.push_meta(format!("{command} failed: {failure}"));
                    }
                    _ => {}
                }
            }
            Message::Event { event, body } => self.on_dap_event(id, &event, &body, window, cx),
            Message::StartDebugging { seq, configuration } => {
                if let Some(debug) = self.debug_mut() {
                    if let Some(client) = debug.clients.get(id) {
                        client.respond(seq, "startDebugging", true);
                    }
                    debug.start_child(configuration);
                }
            }
            Message::Failed(why) => self.end_debugging(&why, cx),
            // A child session ending is just its process ending; the
            // session ends with connection 0.
            Message::Closed if id != 0 => {
                if let Some(debug) = self.debug_mut()
                    && debug.active == id
                {
                    debug.active = 0;
                }
            }
            Message::Closed => {
                if let Some(debug) = self.debug_mut() {
                    debug.clients.clear();
                    debug.server = None;
                }
                self.end_debugging("Debugging finished", cx);
            }
        }
        cx.notify();
    }

    fn on_dap_event(
        &mut self,
        id: usize,
        event: &str,
        body: &Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            "initialized" => {
                let Some(key) = self.debug().map(|debug| debug.plan.debugger.key.clone()) else {
                    return;
                };
                let breakpoints = self.breakpoints_for(&key, cx);
                let Some(debug) = self.debug_mut() else {
                    return;
                };
                for (path, lines) in breakpoints {
                    debug.request_to(
                        id,
                        "setBreakpoints",
                        set_breakpoints_arguments(&path, &lines),
                        Pending::Command("Setting breakpoints"),
                    );
                }
                debug.request_to(
                    id,
                    "configurationDone",
                    json!({}),
                    Pending::Command("Starting"),
                );
                debug.configured = true;
                if debug.phase == Phase::Starting {
                    debug.phase = Phase::Running;
                }
            }
            "stopped" => {
                let Some(debug) = self.debug_mut() else {
                    return;
                };
                debug.forget_pause();
                debug.active = id;
                debug.phase = Phase::Paused;
                debug.stop_reason = body["reason"].as_str().map(str::to_string);
                if let Some(thread) = body["threadId"].as_i64() {
                    debug.thread_id = Some(thread);
                }
                if let Some(thread) = debug.thread_id {
                    debug.request(
                        "stackTrace",
                        json!({"threadId": thread, "startFrame": 0, "levels": MAX_FRAMES}),
                        Pending::StackTrace,
                    );
                }
                debug.view = View::Debugger;
                self.runs.panel_open = true;
                // The program usually has focus while it runs (a terminal, a
                // browser, a game window), so a hit breakpoint would go unseen.
                window.activate_window();
                cx.activate(true);
            }
            "continued" => {
                if let Some(debug) = self.debug_mut() {
                    debug.forget_pause();
                    debug.phase = Phase::Running;
                }
                self.clear_execution_line(cx);
            }
            "output" => {
                let stream = match body["category"].as_str() {
                    Some("stdout") | None => Stream::Stdout,
                    Some("stderr") => Stream::Stderr,
                    Some("telemetry") => return,
                    Some(_) => Stream::Meta,
                };
                let text = body["output"].as_str().unwrap_or_default();
                if stream == Stream::Meta && is_chatter(text) {
                    return;
                }
                let Some(session) = self.runs.session.as_mut() else {
                    return;
                };
                let Some(debug) = session.debug.as_mut() else {
                    return;
                };
                let lines = debug.output_lines(text, stream);
                session.lines.extend(lines);
            }
            "exited" => {
                let code = body["exitCode"].as_i64().unwrap_or_default();
                self.push_meta(format!("Process finished with exit code {code}"));
            }
            // A child session's program ended; others may still run.
            "terminated" if id != 0 => {}
            "terminated" => {
                if let Some(debug) = self.debug_mut() {
                    debug.request_to(
                        0,
                        "disconnect",
                        json!({"terminateDebuggee": true}),
                        Pending::Command("Disconnecting"),
                    );
                }
                self.end_debugging("Debugging finished", cx);
            }
            _ => {}
        }
    }

    fn send_launch(&mut self, cx: &mut Context<Self>) {
        let Some(session) = self.runs.session.as_mut() else {
            return;
        };
        let config = session.config.clone();
        let Some(debug) = session.debug.as_mut() else {
            return;
        };
        let mut extra = debug.lsp_values.lock().unwrap().clone();
        if let Some(file) = &debug.file {
            extra.push(("file".into(), json!(file)));
        }
        let arguments =
            debug
                .plan
                .launch_arguments(&config, &debug.root, debug.executable.as_deref(), &extra);
        match arguments {
            Ok(arguments) => debug.request_to(0, "launch", arguments, Pending::Launch),
            Err(why) => self.end_debugging(&why, cx),
        }
    }

    fn on_stack_trace(&mut self, body: &Value, window: &mut Window, cx: &mut Context<Self>) {
        let frames: Vec<Frame> = body["stackFrames"]
            .as_array()
            .map(|frames| {
                frames
                    .iter()
                    .map(|frame| Frame {
                        id: frame["id"].as_i64().unwrap_or_default(),
                        name: frame["name"].as_str().unwrap_or("?").to_string(),
                        path: frame["source"]["path"]
                            .as_str()
                            .map(PathBuf::from)
                            .filter(|path| path.is_file()),
                        line: frame["line"].as_u64().unwrap_or(1) as u32,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Start where there's code to show: a crash often pauses inside
        // the system's libraries.
        let first = frames.iter().position(|f| f.path.is_some()).unwrap_or(0);
        let Some(debug) = self.debug_mut() else {
            return;
        };
        debug.frames = frames;
        if !debug.frames.is_empty() {
            self.select_frame(first, window, cx);
        }
    }

    pub(super) fn select_frame(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(debug) = self.debug_mut() else {
            return;
        };
        let Some(frame) = debug.frames.get(ix).cloned() else {
            return;
        };
        debug.selected_frame = ix;
        debug.scopes.clear();
        debug.children.clear();
        debug.expanded.clear();
        debug.request(
            "scopes",
            json!({"frameId": frame.id}),
            Pending::Scopes(frame.id),
        );
        match frame.path {
            Some(path) => {
                self.show_execution_line(&path, frame.line.saturating_sub(1), window, cx);
                // Keep the keyboard on the debugger's side of things.
                self.focus_main(window, cx);
            }
            None => self.clear_execution_line(cx),
        }
        cx.notify();
    }

    fn on_scopes(&mut self, frame: i64, body: &Value) {
        let Some(debug) = self.debug_mut() else {
            return;
        };
        if debug.frames.get(debug.selected_frame).map(|f| f.id) != Some(frame) {
            return;
        }
        let scopes: Vec<(Variable, bool)> = body["scopes"]
            .as_array()
            .map(|scopes| {
                scopes
                    .iter()
                    .map(|scope| {
                        (
                            Variable {
                                name: scope["name"].as_str().unwrap_or("Scope").to_string(),
                                value: String::new(),
                                kind: None,
                                reference: scope["variablesReference"].as_i64().unwrap_or(0),
                            },
                            scope["expensive"].as_bool().unwrap_or(false),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Open the first cheap scope, usually Locals.
        if let Some((scope, _)) = scopes
            .iter()
            .find(|(s, expensive)| !expensive && s.reference > 0)
        {
            let reference = scope.reference;
            debug.expanded.insert(reference);
            debug.request(
                "variables",
                json!({"variablesReference": reference}),
                Pending::Variables(reference),
            );
        }
        debug.scopes = scopes.into_iter().map(|(scope, _)| scope).collect();
    }

    fn toggle_variable(&mut self, reference: i64, cx: &mut Context<Self>) {
        let Some(debug) = self.debug_mut() else {
            return;
        };
        if !debug.expanded.remove(&reference) {
            debug.expanded.insert(reference);
            if !debug.children.contains_key(&reference) {
                debug.request(
                    "variables",
                    json!({"variablesReference": reference}),
                    Pending::Variables(reference),
                );
            }
        }
        cx.notify();
    }

    fn end_debugging(&mut self, ending: &str, cx: &mut Context<Self>) {
        let Some(session) = self.runs.session.as_mut() else {
            return;
        };
        let Some(debug) = session.debug.as_mut() else {
            return;
        };
        if let Some(line) = debug.flush_output() {
            session.lines.push(line);
        }
        debug.forget_pause();
        debug.phase = Phase::Ended;
        debug.view = View::Console;
        if session.ending.is_none() {
            let elapsed = session.started.elapsed().as_secs_f32();
            session
                .lines
                .push(OutputLine::meta(format!("{ending} ({elapsed:.1}s)")));
            session.ending = Some(ending.into());
        }
        self.clear_execution_line(cx);
    }

    fn push_meta(&mut self, text: String) {
        if let Some(session) = self.runs.session.as_mut() {
            session.lines.push(OutputLine::meta(text));
        }
    }

    /// A breakpoint changed in `path`: tell a running debugger.
    pub(super) fn breakpoints_changed(&mut self, path: &std::path::Path, cx: &App) {
        let lines = self.breakpoints_in(path, cx);
        if let Some(debug) = self.debug_mut().filter(|d| d.configured && d.is_live()) {
            for id in 0..debug.clients.len() {
                debug.request_to(
                    id,
                    "setBreakpoints",
                    set_breakpoints_arguments(path, &lines),
                    Pending::Command("Setting breakpoints"),
                );
            }
        }
    }

    /// Stop debugging: ask the debugger to end the program.
    pub(super) fn stop_debugging(&mut self) -> bool {
        let Some(debug) = self
            .debug_mut()
            .filter(|d| !d.clients.is_empty() && d.is_live())
        else {
            return false;
        };
        // The session's own connection ends everything it started.
        debug.request_to(
            0,
            "disconnect",
            json!({"terminateDebuggee": true}),
            Pending::Command("Stopping"),
        );
        true
    }

    /// Resume or step the paused thread.
    fn control(&mut self, command: &'static str, cx: &mut Context<Self>) {
        let Some(debug) = self.debug_mut().filter(|d| d.phase == Phase::Paused) else {
            return;
        };
        let Some(thread) = debug.thread_id else {
            return;
        };
        debug.request(
            command,
            json!({"threadId": thread}),
            Pending::Command("Stepping"),
        );
        debug.forget_pause();
        debug.phase = Phase::Running;
        self.clear_execution_line(cx);
        cx.notify();
    }

    pub(super) fn resume(&mut self, _: &Resume, _: &mut Window, cx: &mut Context<Self>) {
        self.control("continue", cx);
    }

    pub(super) fn step_over(&mut self, _: &StepOver, _: &mut Window, cx: &mut Context<Self>) {
        self.control("next", cx);
    }

    pub(super) fn step_into(&mut self, _: &StepInto, _: &mut Window, cx: &mut Context<Self>) {
        self.control("stepIn", cx);
    }

    pub(super) fn step_out(&mut self, _: &StepOut, _: &mut Window, cx: &mut Context<Self>) {
        self.control("stepOut", cx);
    }

    /// Resume, step and view buttons for the panel's header.
    pub(super) fn render_debug_buttons(&self, cx: &Context<Self>) -> Vec<AnyElement> {
        let Some(debug) = self.debug() else {
            return Vec::new();
        };
        let theme = cx.theme();
        let paused = debug.phase == Phase::Paused;
        // Labelled on hover with what they do and their shortcut.
        let button = |id: &'static str,
                      icon: &'static [u8],
                      color: Hsla,
                      enabled: bool,
                      label: &'static str,
                      action: &'static dyn Action| {
            div()
                .id(id)
                .tooltip(move |window, cx| {
                    Tooltip::new(label).action(action, None).build(window, cx)
                })
                .flex_none()
                .size(px(22.))
                .rounded(px(5.))
                .flex()
                .items_center()
                .justify_center()
                .when(enabled, |this| {
                    this.hover(|s| s.bg(theme.foreground.opacity(0.08)))
                })
                .when(!enabled, |this| this.opacity(0.35))
                .child(Icon::default().data(icon).size(px(12.)).text_color(color))
        };
        let tint = theme.blue;
        let mut buttons = vec![
            button(
                "debug-resume",
                RESUME,
                theme.success,
                paused,
                "Continue",
                &Resume,
            )
            .on_click(cx.listener(|this, _, window, cx| this.resume(&Resume, window, cx)))
            .into_any_element(),
            button(
                "debug-step-over",
                STEP_OVER,
                tint,
                paused,
                "Step Over",
                &StepOver,
            )
            .on_click(cx.listener(|this, _, window, cx| this.step_over(&StepOver, window, cx)))
            .into_any_element(),
            button(
                "debug-step-into",
                STEP_INTO,
                tint,
                paused,
                "Step Into",
                &StepInto,
            )
            .on_click(cx.listener(|this, _, window, cx| this.step_into(&StepInto, window, cx)))
            .into_any_element(),
            button(
                "debug-step-out",
                STEP_OUT,
                tint,
                paused,
                "Step Out",
                &StepOut,
            )
            .on_click(cx.listener(|this, _, window, cx| this.step_out(&StepOut, window, cx)))
            .into_any_element(),
        ];
        let tab = |id: &'static str, label: &'static str, view: View| {
            let active = debug.view == view;
            div()
                .id(id)
                .px_2()
                .h(px(20.))
                .flex()
                .items_center()
                .rounded(px(5.))
                .text_size(px(11.5))
                .when(active, |this| {
                    this.bg(theme.foreground.opacity(0.08))
                        .text_color(theme.foreground)
                })
                .when(!active, |this| {
                    this.text_color(theme.muted_foreground)
                        .hover(|s| s.text_color(theme.foreground))
                })
                .child(label)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(debug) = this.debug_mut() {
                        debug.view = view;
                    }
                    cx.notify();
                }))
        };
        buttons.push(
            h_flex()
                .ml_2()
                .gap_0p5()
                .child(tab("debug-view-debugger", "Debugger", View::Debugger))
                .child(tab("debug-view-console", "Console", View::Console))
                .into_any_element(),
        );
        buttons
    }

    /// The Debugger view: the call stack beside the variables.
    pub(super) fn render_debugger(&self, cx: &Context<Self>) -> Option<AnyElement> {
        let debug = self.debug().filter(|d| d.view == View::Debugger)?;
        let theme = cx.theme();
        let message = |text: &'static str| {
            div()
                .p_3()
                .text_size(px(12.))
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element()
        };
        if debug.phase != Phase::Paused {
            let text = match debug.phase {
                Phase::Building | Phase::Starting => "Starting…",
                Phase::Running => "Running. The call stack and variables show when it pauses.",
                _ => "Not paused.",
            };
            return Some(message(text));
        }
        let frames = uniform_list(
            "debug-frames",
            debug.frames.len(),
            cx.processor(|this, range: Range<usize>, _, cx| {
                range
                    .map(|ix| this.render_frame(ix, cx))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&debug.frames_scroll)
        .size_full();
        let rows = variable_rows(debug).len();
        let variables = uniform_list(
            "debug-variables",
            rows,
            cx.processor(|this, range: Range<usize>, _, cx| {
                let Some(debug) = this.debug() else {
                    return Vec::new();
                };
                let rows = variable_rows(debug);
                range
                    .filter_map(|ix| rows.get(ix).map(|row| this.render_variable(ix, row, cx)))
                    .collect::<Vec<_>>()
            }),
        )
        .track_scroll(&debug.variables_scroll)
        .size_full();
        Some(
            h_flex()
                .flex_1()
                .min_h_0()
                .items_stretch()
                .text_size(px(12.))
                .child(
                    div()
                        .w(relative(0.38))
                        .flex_none()
                        .h_full()
                        .py_1()
                        .border_r_1()
                        .border_color(theme.title_bar_border)
                        .child(frames),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .py_1()
                        .font_family(theme.mono_font_family.clone())
                        .child(variables),
                )
                .into_any_element(),
        )
    }

    fn render_frame(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(debug) = self.debug() else {
            return div().into_any_element();
        };
        let frame = &debug.frames[ix];
        let selected = ix == debug.selected_frame;
        let location = frame.path.as_ref().map(|path| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            format!("{name}:{}", frame.line)
        });
        h_flex()
            .id(("debug-frame", ix))
            .h(px(ROW_HEIGHT))
            .px_3()
            .gap_2()
            .overflow_hidden()
            .when(selected, |this| this.bg(theme.blue.opacity(0.16)))
            .when(!selected, |this| {
                this.hover(|s| s.bg(theme.foreground.opacity(0.05)))
            })
            .text_color(if frame.path.is_some() {
                theme.foreground
            } else {
                theme.muted_foreground
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(frame.name.clone()),
            )
            .children(location.map(|location| {
                div()
                    .flex_none()
                    .text_color(theme.muted_foreground)
                    .child(location)
            }))
            .on_click(cx.listener(move |this, _, window, cx| this.select_frame(ix, window, cx)))
            .into_any_element()
    }

    fn render_variable(&self, ix: usize, row: &VariableRow, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let variable = &row.variable;
        let reference = variable.reference;
        let scope = row.depth == 0;
        let chevron = (reference > 0).then(|| {
            Icon::default()
                .data(if row.expanded {
                    CHEVRON_DOWN
                } else {
                    CHEVRON_RIGHT
                })
                .size(px(10.))
                .text_color(theme.muted_foreground)
        });
        h_flex()
            .id(("debug-variable", ix))
            .h(px(ROW_HEIGHT))
            .pl(px(10. + row.depth.saturating_sub(1) as f32 * 14.))
            .pr_3()
            .gap_1()
            .overflow_hidden()
            .whitespace_nowrap()
            .hover(|s| s.bg(theme.foreground.opacity(0.05)))
            .child(div().flex_none().w(px(12.)).children(chevron))
            .child(
                div()
                    .flex_none()
                    .text_color(if scope {
                        theme.muted_foreground
                    } else {
                        theme.foreground
                    })
                    .when(scope, |this| this.font_weight(FontWeight::SEMIBOLD))
                    .child(variable.name.clone()),
            )
            .when(!scope, |this| {
                this.child(
                    div()
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child("="),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.blue)
                        .child(variable.value.clone()),
                )
                .children(variable.kind.clone().map(|kind| {
                    div()
                        .flex_none()
                        .max_w(px(220.))
                        .truncate()
                        .text_color(theme.muted_foreground)
                        .child(kind)
                }))
            })
            .when(reference > 0, |this| {
                this.on_click(
                    cx.listener(move |this, _, _, cx| this.toggle_variable(reference, cx)),
                )
            })
            .into_any_element()
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

/// The debugger's messages that say nothing the panel doesn't: the setup
/// commands it ran, the exit it reports again as `exited`, and a warning
/// about Rust that isn't so, since Jig loads Rust's formatters.
fn is_chatter(text: &str) -> bool {
    let text = text.trim();
    text.starts_with("Running initCommands")
        || text.starts_with("(lldb) command script import")
        || (text.starts_with("Process ") && text.contains(" exited with status"))
        || text.contains("no plugin for the language \"rust\"")
}

fn set_breakpoints_arguments(path: &std::path::Path, lines: &[u32]) -> Value {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let breakpoints: Vec<Value> = lines.iter().map(|line| json!({"line": line + 1})).collect();
    json!({
        "source": {"name": name, "path": path},
        "breakpoints": breakpoints,
        "sourceModified": false,
    })
}

fn parse_variables(body: &Value) -> Vec<Variable> {
    body["variables"]
        .as_array()
        .map(|variables| {
            variables
                .iter()
                .map(|variable| Variable {
                    name: variable["name"].as_str().unwrap_or("?").to_string(),
                    value: variable["value"].as_str().unwrap_or_default().to_string(),
                    kind: variable["type"]
                        .as_str()
                        .filter(|kind| !kind.is_empty())
                        .map(str::to_string),
                    reference: variable["variablesReference"].as_i64().unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn variable_rows(debug: &DebugSession) -> Vec<VariableRow> {
    fn walk(
        debug: &DebugSession,
        variables: &[Variable],
        depth: usize,
        rows: &mut Vec<VariableRow>,
    ) {
        for variable in variables {
            let expanded = variable.reference > 0 && debug.expanded.contains(&variable.reference);
            rows.push(VariableRow {
                depth,
                variable: variable.clone(),
                expanded,
            });
            if expanded && let Some(children) = debug.children.get(&variable.reference) {
                walk(debug, children, depth + 1, rows);
            }
        }
    }
    let mut rows = Vec::new();
    walk(debug, &debug.scopes, 0, &mut rows);
    rows
}
