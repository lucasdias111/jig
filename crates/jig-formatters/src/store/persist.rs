//! A formatter on disk: finding its program, installing and uninstalling
//! it, and running it on a file's text.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use jig_process::NoWindow as _;
use jig_tools as tools;

use crate::domain::formatters::{Formatter, Registry};

impl Formatter {
    /// The program to run, if it's installed: on the `PATH`, in its own
    /// folder if Jig installed it, or in a `node_modules/.bin` above
    /// `file`, where Prettier usually is.
    pub fn program(&self, file: &Path) -> Option<PathBuf> {
        let name = self.command.first()?;
        if name.contains('/') || name.contains('\\') {
            return Some(PathBuf::from(name)).filter(|path| path.is_file());
        }
        let own = self
            .own_dir()
            .map(|own| tools::bin_dirs(&own))
            .into_iter()
            .flatten();
        let local = file
            .ancestors()
            .skip(1)
            .map(|dir| dir.join("node_modules/.bin"));
        std::env::split_paths(tools::search_path())
            .chain(own)
            .chain(local)
            .find_map(|dir| tools::executable_in(&dir, name))
    }

    /// Its own folder in Jig's formatters folder, whether or not it's there.
    fn own_dir(&self) -> Option<PathBuf> {
        formatters_dir().map(|dir| dir.join(&self.key))
    }

    /// Whether Jig installed it, so Uninstall has something to remove.
    pub fn installed_by_jig(&self) -> bool {
        self.own_dir().is_some_and(|dir| dir.is_dir())
    }

    /// What Settings says about it: the program it runs, or how to get it,
    /// and whether it's ready.
    pub fn status(&self) -> (String, bool) {
        match self.program(Path::new("/")) {
            Some(program) => (format!("Using {}", tools::home_relative(&program)), true),
            None if self.installable() => (
                format!("{} isn't installed. Jig can install it.", self.name),
                false,
            ),
            None => (
                format!("{} isn't installed. {}", self.name, self.help()),
                false,
            ),
        }
    }

    /// Run its install command. Blocks, so run it off the main thread.
    pub fn install(&self) -> Result<()> {
        let script = self.install.as_deref().context("Nothing to install")?;
        let dir = self.own_dir().context("No home folder to install into")?;
        tools::install_own_folder(script, &dir)?;
        if self.program(Path::new("/")).is_none() {
            bail!(
                "The installer finished, but Jig still can't find {}.",
                self.name
            );
        }
        Ok(())
    }

    /// Move the folder Jig installed it into to the Trash.
    pub fn uninstall(&self) -> Result<()> {
        let Some(dir) = self.own_dir().filter(|dir| dir.is_dir()) else {
            bail!(
                "Jig didn't install {}, so there's nothing to remove.",
                self.name
            );
        };
        trash::delete(&dir).context("Couldn't move it to the Trash")
    }

    /// `text` formatted, or why not. Gives up, stopping the formatter,
    /// after `limit`.
    pub fn format(
        &self,
        text: &str,
        file: &Path,
        root: &Path,
        limit: Duration,
    ) -> Result<String, String> {
        let program = self
            .program(file)
            .ok_or_else(|| format!("{} isn't installed.", self.name))?;
        let mut command = Command::new(program);
        command
            .no_window()
            .args(self.args(file, root))
            .env("PATH", tools::search_path());
        if let Some(dir) = file.parent().filter(|dir| dir.is_dir()) {
            command.current_dir(dir);
        }
        let output =
            run(command, text, limit).map_err(|error| format!("{}: {error}", self.name))?;
        // Some formatters write `\n` whatever they're given.
        if text.contains("\r\n") && !output.contains("\r\n") {
            return Ok(output.replace('\n', "\r\n"));
        }
        Ok(output)
    }
}

impl Registry {
    /// The first installed formatter for `language`, for `file`.
    pub fn for_language(&self, language: &str, file: &Path) -> Option<std::sync::Arc<Formatter>> {
        self.serving(language)
            .find(|f| f.program(file).is_some())
            .cloned()
    }
}

/// Where Jig keeps formatters it installs, beside its debuggers and
/// language servers. `JIG_FORMATTERS_DIR` says otherwise.
pub fn formatters_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("JIG_FORMATTERS_DIR") {
        return Some(PathBuf::from(dir));
    }
    tools::debuggers_dir()?
        .parent()
        .map(|dir| dir.join("formatters"))
}

/// The user's own `formatters.toml`; `None` in tests.
pub fn user_path() -> Option<PathBuf> {
    tools::user_file("formatters.toml")
}

/// Run `command` with `input` on its stdin and return its stdout, or why
/// it failed. Stopped after `limit`.
fn run(mut command: Command, input: &str, limit: Duration) -> Result<String, String> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("couldn't start: {error}"))?;
    // Written and read on threads of their own, so a formatter that
    // writes before it has read everything can't block on a full pipe.
    let blank = input.trim().is_empty();
    let mut stdin = child.stdin.take().expect("piped");
    let input = input.to_string();
    std::thread::spawn(move || stdin.write_all(input.as_bytes()));
    let read = |mut pipe: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            bytes
        })
    };
    let stdout = read(Box::new(child.stdout.take().expect("piped")));
    let stderr = read(Box::new(child.stderr.take().expect("piped")));
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < limit => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("no answer in {} s", limit.as_secs()));
            }
            Err(error) => return Err(error.to_string()),
        }
    };
    let stdout = stdout.join().unwrap_or_default();
    let stderr = stderr.join().unwrap_or_default();
    if !status.success() {
        let stderr = String::from_utf8_lossy(&stderr);
        let reason = stderr
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .map_or_else(|| format!("failed ({status})"), str::to_string);
        return Err(reason);
    }
    let output = String::from_utf8(stdout).map_err(|_| "wrote text that isn't UTF-8")?;
    // Nothing back for something sent is a failure, not an empty file.
    if output.is_empty() && !blank {
        return Err("returned nothing".into());
    }
    Ok(output)
}
