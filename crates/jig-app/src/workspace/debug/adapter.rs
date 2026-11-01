//! Starting a debugger's adapter, on its own or inside a language server, and what it's told first.

use super::*;

impl DebugSession {
    pub(in crate::workspace) fn is_live(&self) -> bool {
        self.phase != Phase::Ended
    }

    pub(in crate::workspace) fn status(&self) -> Option<String> {
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
    pub(super) fn request(&mut self, command: &'static str, arguments: Value, pending: Pending) {
        self.request_to(self.active, command, arguments, pending);
    }

    pub(super) fn request_to(
        &mut self,
        id: usize,
        command: &'static str,
        arguments: Value,
        pending: Pending,
    ) {
        if let Some(client) = self.clients.get(id) {
            let seq = client.request(command, arguments);
            self.pending.insert((id, seq), pending);
        }
    }

    /// Start a child session on the adapter server, as it asked.
    pub(super) fn start_child(&mut self, configuration: Value) {
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
    pub(super) fn forget_pause(&mut self) {
        self.frames.clear();
        self.scopes.clear();
        self.children.clear();
        self.expanded.clear();
        self.selected_frame = 0;
        self.stop_reason = None;
    }

    /// Output as lines. The program's may come in pieces, so a trailing
    /// part waits for its newline; the debugger's own messages are whole.
    pub(super) fn output_lines(&mut self, text: &str, stream: Stream) -> Vec<OutputLine> {
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
            lines.push(jig_run::run_output::parse_line(
                &line,
                stream,
                &mut self.style,
                &[],
            ));
        }
        lines
    }

    pub(super) fn flush_output(&mut self) -> Option<OutputLine> {
        (!self.partial.is_empty()).then(|| {
            let line = std::mem::take(&mut self.partial);
            jig_run::run_output::parse_line(&line, self.partial_stream, &mut self.style, &[])
        })
    }
}

/// Start `debugger`'s adapter in `cwd` and connect to it, as connection 0.
pub(super) fn start_adapter(
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
pub(super) fn start_lsp_adapter(
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
        .find(|language| jig_lsp::lsp::server_for(language).is_some_and(|s| s.name == server))
        .ok_or_else(|| anyhow::anyhow!("Jig doesn't run {server} for any of its languages."))?;
    let client = jig_lsp::lsp::client_for(&root.join("_"), language, cx)
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
        let started = jig_debug::lsp_debug::start(&client, &server, &start, &root, &prefer);
        let _ = port_tx.send(started.map(|started| {
            *values.lock().unwrap() = started.values;
            started.port
        }));
    });
    Ok(dap::Client::connect_when(0, port_rx, inbox))
}

#[cfg(test)]
pub(super) fn faked() -> bool {
    FAKE_ADAPTER.with(|fake| fake.borrow().is_some())
}

#[cfg(not(test))]
pub(super) fn faked() -> bool {
    false
}

pub(super) fn initialize_arguments(adapter_id: &str) -> Value {
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
