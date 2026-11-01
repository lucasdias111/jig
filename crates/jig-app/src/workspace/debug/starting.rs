//! Starting a debug session: building the program first where the debugger asks for it, then launching.

use super::*;

impl Workspace {
    /// ⌃D: debug the selected configuration, or choose one to.
    pub(in crate::workspace) fn debug_selected(
        &mut self,
        _: &DebugSelected,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.run_configs.read(cx).selected().cloned() {
            Some(config) => self.start_debug(config, window, cx),
            None => self.open_run_picker_for(true, window, cx),
        }
    }

    pub(in crate::workspace) fn start_debug(
        &mut self,
        config: RunConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_map_debug(cx);
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
    pub(in crate::workspace) fn edit_debuggers(
        &mut self,
        _: &EditDebuggers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit_user_file(debuggers::user_path(), debuggers::USER_TEMPLATE, window, cx);
    }

    /// Cargo's JSON messages while building: the one naming the executable.
    pub(in crate::workspace) fn on_build_data(&mut self, json: &str) {
        if let Some(debug) = self.debug_mut()
            && let Some(executable) = debug_target::cargo_executable(json)
        {
            debug.executable = Some(executable);
        }
    }

    /// The build finished: debug what it built, or say why not. `stopped`
    /// when it was stopped with ⌘F2.
    pub(in crate::workspace) fn on_build_exit(
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
            self.run_output
                .update(cx, |o, cx| o.push(OutputLine::meta(failure), cx));
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
                let line = OutputLine::meta(format!("Waiting for {server} to load the project…"));
                self.run_output.update(cx, |o, cx| o.push(line, cx));
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
                let line = OutputLine::meta(format!("Debugging {what}"));
                self.run_output.update(cx, |o, cx| o.push(line, cx));
                let arguments = initialize_arguments(debugger.adapter_id());
                debug.request_to(0, "initialize", arguments, Pending::Initialize);
            }
            Err(error) => {
                let message = format!("{error:#}");
                debug.phase = Phase::Ended;
                let line = OutputLine::meta(message.clone());
                self.run_output.update(cx, |o, cx| o.push(line, cx));
                session.ending = Some(message);
            }
        }
        cx.notify();
    }

    pub(super) fn send_launch(&mut self, cx: &mut Context<Self>) {
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
}
