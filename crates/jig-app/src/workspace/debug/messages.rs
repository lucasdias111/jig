//! What the adapter says: events, answers to requests, the stack and the variables.

use super::*;

impl Workspace {
    /// What the adapter said since last time.
    pub(in crate::workspace) fn take_dap_messages(&mut self) -> Vec<(usize, Message)> {
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
    pub(in crate::workspace) fn on_dap_message(
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
                        self.push_meta(format!("A child session couldn't start: {failure}"), cx);
                    }
                    Pending::StackTrace if success => self.on_stack_trace(&body, window, cx),
                    Pending::Scopes(frame) if success => self.on_scopes(frame, &body),
                    Pending::Variables(reference) if success => {
                        if let Some(debug) = self.debug_mut() {
                            debug.children.insert(reference, parse_variables(&body));
                        }
                    }
                    Pending::Command(command) if !success => {
                        self.push_meta(format!("{command} failed: {failure}"), cx);
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
                let mut breakpoints = self.breakpoints_for(&key, cx);
                // Files only the Map wants to hear from.
                let live: Vec<PathBuf> = self
                    .live
                    .files()
                    .filter(|path| breakpoints.iter().all(|(file, _)| file != *path))
                    .filter(|path| self.debugger_takes(&key, path))
                    .cloned()
                    .collect();
                breakpoints.extend(live.into_iter().map(|path| (path, Vec::new())));
                let requests: Vec<Value> = breakpoints
                    .iter()
                    .map(|(path, lines)| {
                        let logpoints = self.live.logpoints(path, lines);
                        set_breakpoints_arguments(path, lines, &logpoints)
                    })
                    .collect();
                let Some(debug) = self.debug_mut() else {
                    return;
                };
                for arguments in requests {
                    debug.request_to(
                        id,
                        "setBreakpoints",
                        arguments,
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
                if let Some(ids) = super::super::live::fired(text) {
                    self.on_fired(&ids, cx);
                    return;
                }
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
                self.run_output.update(cx, |o, cx| o.extend(lines, cx));
            }
            "exited" => {
                let code = body["exitCode"].as_i64().unwrap_or_default();
                self.push_meta(format!("Process finished with exit code {code}"), cx);
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
        self.refresh_map_debug(cx);
    }

    pub(in crate::workspace) fn select_frame(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    pub(super) fn toggle_variable(&mut self, reference: i64, cx: &mut Context<Self>) {
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

    pub(super) fn end_debugging(&mut self, ending: &str, cx: &mut Context<Self>) {
        let Some(session) = self.runs.session.as_mut() else {
            return;
        };
        let Some(debug) = session.debug.as_mut() else {
            return;
        };
        if let Some(line) = debug.flush_output() {
            self.run_output.update(cx, |o, cx| o.push(line, cx));
        }
        debug.forget_pause();
        debug.phase = Phase::Ended;
        debug.view = View::Console;
        if session.ending.is_none() {
            let elapsed = session.started.elapsed().as_secs_f32();
            let line = OutputLine::meta(format!("{ending} ({elapsed:.1}s)"));
            self.run_output.update(cx, |o, cx| o.push(line, cx));
            session.ending = Some(ending.into());
        }
        self.clear_execution_line(cx);
    }

    fn push_meta(&mut self, text: String, cx: &mut Context<Self>) {
        if self.runs.session.is_some() {
            self.run_output
                .update(cx, |o, cx| o.push(OutputLine::meta(text), cx));
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
        || text.strip_prefix("(lldb) ") == Some(jig_debuggers::WITHOUT_STRUCT_SUMMARIES)
        || (text.starts_with("Process ") && text.contains(" exited with status"))
        || text.contains("no plugin for the language \"rust\"")
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
