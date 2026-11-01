//! The agent behind Jig's AI: OpenCode's server or an ACP agent's
//! process, started on first use, shared by every window, and stopped when
//! Jig quits.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui_kit::App;
use jig_ai::agent::acp::{AgentCommandLine, Connection};
use jig_ai::agent::harness::Backend;
use jig_ai::agent::opencode;
use jig_process::NoWindow as _;
use serde_json::Value;

use crate::agents::Kind;

/// The running OpenCode server and the provider config it was started with.
static SERVER: Mutex<Option<(Option<Value>, Arc<opencode::Server>)>> = Mutex::new(None);

/// Running ACP agents by key, with how each was started.
type Running = HashMap<String, (AgentCommandLine, Arc<Connection>)>;
static AGENTS: Mutex<Option<Running>> = Mutex::new(None);

/// An agent and model picked in ⌘K, in place of the settings', for the
/// window. `model` is `provider/model` for OpenCode, the agent's own name
/// for an ACP agent, or `None` for the agent's default.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pick {
    pub agent: String,
    pub model: Option<String>,
}

impl Pick {
    /// What the settings say: the agent picked there, on its model.
    pub fn from_settings(cx: &App) -> Self {
        let agent = crate::agents::chosen(cx);
        let model = match agent.kind {
            Kind::OpenCode => crate::providers::config(cx)
                .ok()
                .and_then(|config| config.agent.clone().filter(|id| !id.is_empty())),
            Kind::Acp => jig_settings::get(cx).agent.models.get(&agent.key).cloned(),
        };
        Self {
            agent: agent.key.clone(),
            model,
        }
    }
}

/// Which agent to run and how, read on the main thread, where the settings
/// and keys are.
#[derive(Clone, Debug)]
pub enum Choice {
    OpenCode(opencode::Target),
    Acp {
        key: String,
        name: String,
        command: AgentCommandLine,
        /// Its model, by its own name; `None` for its default.
        model: Option<String>,
    },
}

impl Choice {
    /// The model to ask for, as OpenCode names it; ACP agents pick theirs
    /// as the conversation starts.
    pub fn model(&self) -> String {
        match self {
            Choice::OpenCode(target) => target.model.clone(),
            Choice::Acp { .. } => String::new(),
        }
    }

    /// An ACP agent's model, by its own name.
    pub fn acp_model(&self) -> Option<&str> {
        match self {
            Choice::OpenCode(_) => None,
            Choice::Acp { model, .. } => model.as_deref(),
        }
    }

    /// The agent's key, for what's remembered about it.
    pub fn key(&self) -> &str {
        match self {
            Choice::OpenCode(_) => crate::agents::DEFAULT,
            Choice::Acp { key, .. } => key,
        }
    }
}

/// The agent `pick` names, else the one Settings picked, ready to start.
pub fn choice_for(pick: Option<&Pick>, cx: &App) -> Result<Choice, String> {
    let pick = pick.cloned().unwrap_or_else(|| Pick::from_settings(cx));
    let agent = crate::agents::named(&pick.agent);
    match agent.kind {
        Kind::OpenCode => {
            crate::providers::agent_target_for(pick.model.as_deref(), cx).map(Choice::OpenCode)
        }
        Kind::Acp => Ok(Choice::Acp {
            key: agent.key.clone(),
            name: agent.name.clone(),
            command: agent
                .command_line()
                .map_err(|missing| format!("{missing} Install it in Settings > Agent."))?,
            model: pick.model,
        }),
    }
}

/// Stop the servers on quit; a static is never dropped on its own.
pub fn init(cx: &mut App) {
    cx.on_app_quit(|_| {
        stop();
        async {}
    })
    .detach();
}

/// Stop every running agent.
pub fn stop() {
    if let Some((_, server)) = SERVER.lock().ok().and_then(|mut server| server.take()) {
        server.stop();
    }
    if let Some(agents) = AGENTS.lock().ok().and_then(|mut agents| agents.take()) {
        for (_, connection) in agents.values() {
            connection.stop();
        }
    }
}

/// The agent for `choice`, starting it if needed. One started another way
/// (other providers, another command) is replaced; runs still using it keep
/// it until they end. Blocks while it starts, so call it off the main thread.
pub fn connect(choice: &Choice) -> anyhow::Result<Backend> {
    match choice {
        Choice::OpenCode(target) => server(target.config.as_ref()).map(Backend::OpenCode),
        Choice::Acp {
            key, name, command, ..
        } => {
            let mut agents = AGENTS.lock().unwrap_or_else(|error| error.into_inner());
            let agents = agents.get_or_insert_with(HashMap::new);
            if let Some((started_with, connection)) = agents.get(key)
                && started_with == command
                && connection.alive()
            {
                return Ok(Backend::Acp(connection.clone()));
            }
            let connection = Connection::start(name, command)?;
            agents.insert(key.clone(), (command.clone(), connection.clone()));
            Ok(Backend::Acp(connection))
        }
    }
}

/// Ask an ACP agent which models it offers, by starting it and opening an
/// empty conversation, which agents don't keep until something is said in
/// it. Blocks, so call it off the main thread.
pub fn look_up_models(choice: &Choice) -> anyhow::Result<Option<jig_ai::agent::acp::AgentModels>> {
    let Choice::Acp { .. } = choice else {
        return Ok(None);
    };
    let backend = connect(choice)?;
    if let Backend::Acp(connection) = &backend {
        jig_ai::agent::acp::Session::create(connection.clone(), &std::env::temp_dir(), None)?;
    }
    Ok(backend.models())
}

/// The OpenCode server started with `config`, starting one if needed.
fn server(config: Option<&Value>) -> anyhow::Result<Arc<opencode::Server>> {
    let mut server = SERVER.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((started_with, server)) = server.as_ref()
        && started_with.as_ref() == config
    {
        return Ok(server.clone());
    }
    let started = Arc::new(opencode::Server::start(config)?);
    *server = Some((config.cloned(), started.clone()));
    Ok(started)
}

/// Open the system's terminal running `command`, for an agent's own login.
/// The window stays open when it's done, saying to go back to Jig.
pub fn open_in_terminal(command: &AgentCommandLine) -> anyhow::Result<()> {
    use anyhow::Context as _;
    let quote = |word: &str| format!("'{}'", word.replace('\'', r"'\''"));
    let line = std::iter::once(command.program.to_string_lossy().into_owned())
        .chain(command.args.iter().cloned())
        .map(|word| quote(&word))
        .collect::<Vec<_>>()
        .join(" ");
    let mut script = String::from("#!/bin/sh\n");
    if let Some(path) = &command.path {
        script.push_str(&format!("export PATH={}\n", quote(&path.to_string_lossy())));
    }
    for (key, value) in &command.env {
        script.push_str(&format!("export {key}={}\n", quote(value)));
    }
    script.push_str(&format!(
        "{line}\necho\necho 'Done. Close this window and go back to Jig.'\n"
    ));
    if cfg!(windows) {
        let mut words = vec![
            "/c".to_string(),
            "start".into(),
            String::new(),
            "cmd".into(),
            "/k".into(),
        ];
        words.push(command.program.to_string_lossy().into_owned());
        words.extend(command.args.iter().cloned());
        std::process::Command::new("cmd")
            .no_window()
            .args(words)
            .envs(command.env.iter().map(|(k, v)| (k, v)))
            .spawn()
            .context("Couldn't open a terminal")?;
        return Ok(());
    }
    let file = std::env::temp_dir().join(if cfg!(target_os = "macos") {
        "jig-sign-in.command"
    } else {
        "jig-sign-in.sh"
    });
    std::fs::write(&file, script).context("Couldn't write the sign-in script")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o700))?;
    }
    if cfg!(target_os = "macos") {
        // A .command file opens in the user's terminal app.
        std::process::Command::new("open")
            .arg(&file)
            .spawn()
            .context("Couldn't open a terminal")?;
        return Ok(());
    }
    let terminals: [(&str, &[&str]); 4] = [
        ("x-terminal-emulator", &["-e"]),
        ("gnome-terminal", &["--"]),
        ("konsole", &["-e"]),
        ("xterm", &["-e"]),
    ];
    for (terminal, flags) in terminals {
        if std::process::Command::new(terminal)
            .args(flags)
            .arg(&file)
            .spawn()
            .is_ok()
        {
            return Ok(());
        }
    }
    anyhow::bail!("Couldn't find a terminal. Run this in one: {line}")
}
