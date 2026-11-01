//! Running a configuration: saving first, starting it, polling its output and stopping it.

use super::*;

impl Workspace {
    pub(in crate::workspace) fn stop_run(
        &mut self,
        _: &StopRun,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.stop_process();
        cx.notify();
    }

    pub(in crate::workspace) fn stop_process(&mut self) {
        if self.stop_debugging() {
            return;
        }
        if let Some(process) = self
            .runs
            .session
            .as_ref()
            .and_then(|session| session.process.as_ref())
        {
            process.stop();
        }
    }

    pub(in crate::workspace) fn start_run(
        &mut self,
        config: RunConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_session(config.clone(), Some(config), false, None, window, cx);
    }

    /// Start a session for `config`, running `process` (the program, or a
    /// debug session's build) if given. With `json_stdout`, the process's
    /// JSON lines are read as data.
    pub(in crate::workspace) fn start_session(
        &mut self,
        config: RunConfig,
        process: Option<RunConfig>,
        json_stdout: bool,
        debug: Option<super::super::debug::DebugSession>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.save_all_for_run(window, cx);
        // Stopped by being dropped.
        self.runs.session = None;
        self.clear_execution_line(cx);
        self.runs.next_id += 1;
        let id = self.runs.next_id;
        let root = self.run_root(cx).unwrap_or_else(|| config.cwd.clone());

        let (sender, events) = futures::channel::mpsc::unbounded();
        let mut lines = Vec::new();
        let mut ending = None;
        let process = process.and_then(|process| {
            lines.push(OutputLine::meta(format!("$ {}", process.command)));
            match Process::spawn(&process, &root, json_stdout, sender) {
                Ok(process) => Some(process),
                Err(error) => {
                    let message = format!("{error:#}");
                    lines.push(OutputLine::meta(message.clone()));
                    ending = Some(message);
                    None
                }
            }
        });
        let mut debug = debug;
        if ending.is_some()
            && let Some(debug) = debug.as_mut()
        {
            debug.phase = super::super::debug::Phase::Ended;
        }
        // Polled rather than awaited: output comes from the process's and
        // the debugger's own threads, and a timer batches a flood of it
        // into fewer repaints.
        let poll = cx.spawn_in(window, async move |this, cx| {
            loop {
                let busy = this.update_in(cx, |this, window, cx| this.poll_session(id, window, cx));
                match busy {
                    Ok(Some(true)) => {}
                    Ok(Some(false)) => cx.background_executor().timer(POLL_INTERVAL).await,
                    Ok(None) | Err(_) => break,
                }
            }
        });
        self.run_output
            .update(cx, |output, cx| output.start(lines, cx));
        self.runs.session = Some(RunSession {
            id,
            config,
            process,
            events,
            ending,
            started: Instant::now(),
            debug,
            _poll: poll,
        });
        self.runs.panel_open = true;
        cx.notify();
    }

    /// Take in what the process and the debugger said. `None` once session
    /// `id` is over; `Some(true)` when there was more than one batch's worth.
    fn poll_session(
        &mut self,
        id: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<bool> {
        let session = self.runs.session.as_mut().filter(|s| s.id == id)?;
        let mut events = Vec::new();
        while events.len() < MAX_BATCH {
            match session.events.try_recv() {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty | TryRecvError::Closed) => break,
            }
        }
        let full = events.len() == MAX_BATCH;
        let follow = self.run_output.read(cx).at_end();
        let before = self.run_output.read(cx).len();
        for event in events {
            self.on_run_event(event, window, cx);
        }
        let messages = self.take_dap_messages();
        let any = !messages.is_empty();
        for (id, message) in messages {
            self.on_dap_message(id, message, window, cx);
        }
        let session = self.runs.session.as_mut().filter(|s| s.id == id)?;
        self.run_output.update(cx, |output, cx| output.trim(cx));
        let output = self.run_output.read(cx);
        if output.len() != before || any {
            if follow {
                output.scroll_to_end();
            }
            cx.notify();
        }
        // Done when it has ended and nothing is left to read.
        let live = session.ending.is_none() || session.process.is_some();
        live.then_some(full)
    }

    fn on_run_event(&mut self, event: RunEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(session) = self.runs.session.as_mut() else {
            return;
        };
        match event {
            RunEvent::Line(line) => self.run_output.update(cx, |o, cx| o.push(line, cx)),
            RunEvent::Data(json) => self.on_build_data(&json),
            RunEvent::Exited { message, success } => {
                if session.debug.is_some() {
                    let stopped = message == "Process stopped";
                    if success {
                        self.run_output
                            .update(cx, |o, cx| o.push(OutputLine::meta("Build finished"), cx));
                    } else if !stopped {
                        self.run_output
                            .update(cx, |o, cx| o.push(OutputLine::meta(message), cx));
                    }
                    self.on_build_exit(success, stopped, window, cx);
                    return;
                }
                session.process = None;
                let elapsed = session.started.elapsed().as_secs_f32();
                let line = OutputLine::meta(format!("{message} ({elapsed:.1}s)"));
                self.run_output.update(cx, |o, cx| o.push(line, cx));
                session.ending = Some(message);
            }
        }
    }

    /// Save every changed file that has somewhere to go.
    fn save_all_for_run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut saved_active = false;
        let mut failed = Vec::new();
        for ix in 0..self.tabs.len() {
            let tab = &self.tabs[ix];
            let Some(path) = tab.document.path.clone().filter(|_| tab.dirty) else {
                continue;
            };
            let text = tab.editor.text(cx);
            match self.tabs[ix].document.save(&path, &text) {
                Ok(()) => {
                    self.tabs[ix].dirty = false;
                    saved_active |= ix == self.active;
                }
                Err(_) => failed.push(self.tabs[ix].document.title()),
            }
        }
        if saved_active {
            self.lsp_saved(cx);
        }
        self.update_title(window);
        if !failed.is_empty() {
            self.show_error(
                &format!("Couldn't save {}; running anyway.", failed.join(", ")),
                window,
                cx,
            );
        }
    }

    /// A link in the output was clicked: open the code there.
    pub(in crate::workspace) fn on_run_output_event(
        &mut self,
        _: &Entity<RunOutput>,
        event: &RunOutputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let RunOutputEvent::OpenLink(link) = event;
        self.open_link(link, window, cx);
    }

    pub(in crate::workspace) fn open_link(
        &mut self,
        link: &Link,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let line = link.line.saturating_sub(1);
        let column = link.column.unwrap_or(1).saturating_sub(1);
        let position = lsp_types::Position::new(line, column);
        self.open_at(
            &link.path,
            lsp_types::Range::new(position, position),
            window,
            cx,
        );
    }
}
