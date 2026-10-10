//! The agents Agent mode can run, as data: `assets/default-agents.toml` has
//! the built-ins and documents every field, and the user's
//! `~/.config/jig/agents.toml` adds more or replaces one by `key`.
//! Settings > Agent picks one (`[agent] use`).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use anyhow::{Context as _, Result, bail};
use gpui_kit::App;
use jig_ai::acp::AgentCommandLine;
use serde::Deserialize;

const BUILT_IN: &str = include_str!("../../../assets/default-agents.toml");

/// The agent used when the settings name none, or one that's gone.
pub const DEFAULT: &str = "opencode";

/// A starter for the user's `agents.toml`.
pub const USER_TEMPLATE: &str = r#"# Your agents for Agent mode. Any agent that speaks the Agent Client
# Protocol on stdio works; one with the same key as a built-in replaces it.
# See assets/default-agents.toml in Jig's source for the built-ins and
# every field. Pick one in Settings, under Agent.
#
# [[agent]]
# key = "goose"
# name = "Goose"
# command = ["goose", "acp"]
# about = "Block's Goose."
"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AgentFile {
    #[serde(default)]
    agent: Vec<Agent>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// Jig's own OpenCode client, with the model from Settings > Models.
    OpenCode,
    /// The Agent Client Protocol on stdio.
    #[default]
    Acp,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Agent {
    pub key: String,
    pub name: String,
    #[serde(default)]
    pub kind: Kind,
    /// The program and its arguments.
    command: Vec<String>,
    #[serde(default)]
    env: std::collections::BTreeMap<String, String>,
    install: Option<String>,
    #[serde(default)]
    pub about: String,
}

impl Agent {
    pub fn installable(&self) -> bool {
        self.install.is_some()
    }

    /// The program, where Jig would start it from, or what's missing.
    fn program(&self) -> Result<PathBuf, String> {
        let word = self.command.first().ok_or("It has no command.")?;
        if word.contains('/') {
            let path = PathBuf::from(word);
            return if path.is_file() {
                Ok(path)
            } else {
                Err(format!("Nothing at {word}."))
            };
        }
        std::env::split_paths(&search_path())
            .map(|dir| dir.join(word))
            .find(|path| path.is_file())
            .ok_or_else(|| format!("{} isn't installed.", self.name))
    }

    /// How to start it as an ACP agent.
    pub fn command_line(&self) -> Result<AgentCommandLine, String> {
        Ok(AgentCommandLine {
            program: self.program()?,
            args: self.command[1..].to_vec(),
            env: self
                .env
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            path: Some(search_path()),
        })
    }
}

/// The `PATH` agents are looked for on and started with: the login shell's,
/// with Jig's agents folder.
fn search_path() -> std::ffi::OsString {
    let mut dirs: Vec<PathBuf> = std::env::split_paths(crate::lsp::search_path()).collect();
    if let Some(dir) = agents_dir() {
        dirs.push(dir.join("node_modules/.bin"));
        dirs.push(dir.join("bin"));
    }
    std::env::join_paths(dirs).unwrap_or_default()
}

pub struct Registry {
    pub agents: Vec<Arc<Agent>>,
    /// Why the user's file couldn't be read, if it couldn't.
    pub error: Option<String>,
}

impl Registry {
    pub fn get(&self, key: &str) -> Option<Arc<Agent>> {
        self.agents.iter().find(|agent| agent.key == key).cloned()
    }
}

/// The agents as they are now; the user's file is read again when it has
/// changed.
pub fn registry() -> Arc<Registry> {
    type Cached = (Option<SystemTime>, Arc<Registry>);
    static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
    let path = user_path();
    let modified = path
        .as_deref()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|meta| meta.modified().ok());
    let mut cache = CACHE.lock().unwrap_or_else(|error| error.into_inner());
    if let Some((stamp, registry)) = cache.as_ref()
        && *stamp == modified
    {
        return registry.clone();
    }
    let registry = Arc::new(load(path.as_deref()));
    *cache = Some((modified, registry.clone()));
    registry
}

fn load(user: Option<&Path>) -> Registry {
    let mut agents = parse(BUILT_IN).expect("the built-in agents parse");
    let mut error = None;
    if let Some(path) = user.filter(|path| path.exists()) {
        match std::fs::read_to_string(path)
            .map_err(anyhow::Error::from)
            .and_then(|text| parse(&text))
        {
            Ok(mine) => {
                for agent in mine {
                    agents.retain(|other| other.key != agent.key);
                    agents.push(agent);
                }
            }
            Err(why) => error = Some(format!("agents.toml: {why:#}")),
        }
    }
    Registry {
        agents: agents.into_iter().map(Arc::new).collect(),
        error,
    }
}

fn parse(text: &str) -> Result<Vec<Agent>> {
    let file: AgentFile = toml::from_str(text)?;
    for agent in &file.agent {
        if agent.command.is_empty() {
            bail!("{} has no command", agent.name);
        }
    }
    Ok(file.agent)
}

/// The user's `agents.toml`; `None` in tests.
pub fn user_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/jig/agents.toml"))
}

/// The agent Settings picked, or OpenCode.
pub fn chosen(cx: &App) -> Arc<Agent> {
    let registry = registry();
    registry
        .get(&crate::settings::get(cx).agent.using)
        .or_else(|| registry.get(DEFAULT))
        .expect("OpenCode is built in")
}

/// What Settings says about `agent`, and whether it's ready.
pub fn status(agent: &Agent) -> (String, bool) {
    match agent.program() {
        Ok(path) => (format!("Using {}", home_relative(&path)), true),
        Err(missing) if agent.installable() => (format!("{missing} Jig can install it."), false),
        Err(missing) => (missing, false),
    }
}

/// Run `agent`'s install command. Blocks, so run it off the main thread.
pub fn install(agent: &Agent) -> Result<()> {
    let script = agent.install.as_deref().context("Nothing to install")?;
    let dir = agents_dir().context("No home folder to install into")?;
    std::fs::create_dir_all(&dir).with_context(|| format!("Couldn't create {}", dir.display()))?;
    let script = script.replace("${agents}", &dir.to_string_lossy());
    // Say what's missing rather than letting the shell say "not found".
    if let Some(tool) = script.split_whitespace().next()
        && !tool.contains('/')
        && !std::env::split_paths(&search_path()).any(|dir| dir.join(tool).is_file())
    {
        let get = match tool {
            "npm" | "npx" => " Install Node.js from nodejs.org, then try again.",
            _ => "",
        };
        bail!(
            "Installing {} needs {tool}, which isn't installed.{get}",
            agent.name
        );
    }
    let sh = if cfg!(windows) { "sh" } else { "/bin/sh" };
    let output = Command::new(sh)
        .arg("-c")
        .arg(&script)
        .current_dir(&dir)
        .env("PATH", search_path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("Couldn't run the installer")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let last: Vec<&str> = stderr.trim().lines().rev().take(3).collect();
        let last: Vec<&str> = last.into_iter().rev().collect();
        bail!("Installing failed: {}", last.join(" "));
    }
    Ok(())
}

/// Where Jig installs agents, beside its debuggers.
pub fn agents_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("JIG_AGENTS_DIR") {
        return Some(PathBuf::from(dir));
    }
    crate::debuggers::data_dir().map(|dir| {
        dir.join(if cfg!(target_os = "macos") {
            "Jig"
        } else {
            "jig"
        })
        .join("agents")
    })
}

fn home_relative(path: &Path) -> String {
    let path = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && path.starts_with(&home) => {
            format!("~{}", &path[home.len()..])
        }
        _ => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_ins_parse_with_opencode_first() {
        let registry = load(None);
        let keys: Vec<_> = registry.agents.iter().map(|a| a.key.as_str()).collect();
        assert_eq!(keys, ["opencode", "claude-code", "codex", "gemini"]);
        assert_eq!(registry.get("opencode").unwrap().kind, Kind::OpenCode);
        assert_eq!(registry.get("codex").unwrap().kind, Kind::Acp);
        assert!(registry.agents.iter().all(|agent| agent.installable()));
    }

    #[test]
    fn the_user_s_file_adds_and_replaces() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agents.toml");
        std::fs::write(
            &path,
            "[[agent]]\nkey = \"goose\"\nname = \"Goose\"\ncommand = [\"goose\", \"acp\"]\n\n\
             [[agent]]\nkey = \"codex\"\nname = \"My Codex\"\ncommand = [\"/nowhere/codex-acp\"]\n",
        )
        .unwrap();
        let registry = load(Some(&path));
        assert!(registry.error.is_none());
        assert_eq!(registry.get("codex").unwrap().name, "My Codex");
        let goose = registry.get("goose").unwrap();
        assert_eq!(goose.kind, Kind::Acp);
        assert_eq!(goose.command[1..], ["acp"]);
        assert!(
            status(&registry.get("codex").unwrap())
                .0
                .contains("Nothing at")
        );

        std::fs::write(&path, "[[agent]]\nkey = \"x\"\n").unwrap();
        assert!(
            load(Some(&path)).error.is_some(),
            "a broken file is reported"
        );
    }
}
