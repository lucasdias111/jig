//! The formatters Jig runs when a file's language server doesn't format:
//! `assets/default-formatters.toml` and the user's
//! `~/.config/jig/formatters.toml`, data like the debuggers. Each reads the
//! text on stdin and writes it formatted to stdout. Also the two tidy-ups
//! made on save, trailing whitespace and the final newline.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context as _, Result, bail};
use jig_process::NoWindow as _;
use serde::Deserialize;

use jig_kit::editorconfig::LineEnding;
use jig_tools::{self as tools, Help};

const BUILT_IN: &str = include_str!("../../../assets/default-formatters.toml");

/// A starter for the user's `formatters.toml`.
pub const USER_TEMPLATE: &str = r#"# Your formatters, used when a file's language server doesn't format. One
# with the same key as a built-in replaces it; see
# assets/default-formatters.toml in Jig's source for the built-ins. A
# formatter reads the text on stdin and writes it formatted to stdout;
# ${file} is the file's path and ${root} the project's folder.
#
# [[formatter]]
# key = "clang-format"
# name = "clang-format"
# languages = ["c", "cpp"]
# command = ["clang-format", "--assume-filename", "${file}"]
"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FormatterFile {
    #[serde(default)]
    formatter: Vec<Formatter>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Formatter {
    pub key: String,
    pub name: String,
    /// Jig's names for the languages it formats.
    pub languages: Vec<String>,
    command: Vec<String>,
    /// A shell command installing it into `${dir}`, its own folder in
    /// Jig's formatters folder.
    install: Option<String>,
    help: Option<Help>,
}

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
        std::env::split_paths(jig_tools::search_path())
            .chain(own)
            .chain(local)
            .find_map(|dir| tools::executable_in(&dir, name))
    }

    pub fn installable(&self) -> bool {
        self.install.is_some()
    }

    /// How to get it, for when it isn't installed.
    pub fn help(&self) -> String {
        self.help.as_ref().map_or_else(
            || format!("Install {} and put it on the PATH.", self.name),
            Help::text,
        )
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

    /// The arguments, with `${file}` and `${root}` filled in.
    fn args(&self, file: &Path, root: &Path) -> Vec<String> {
        self.command[1..]
            .iter()
            .map(|arg| {
                arg.replace("${file}", &file.to_string_lossy())
                    .replace("${root}", &root.to_string_lossy())
            })
            .collect()
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
            .env("PATH", jig_tools::search_path());
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

/// Where Jig keeps formatters it installs, beside its debuggers and
/// language servers. `JIG_FORMATTERS_DIR` says otherwise.
pub fn formatters_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("JIG_FORMATTERS_DIR") {
        return Some(PathBuf::from(dir));
    }
    jig_tools::debuggers_dir()?
        .parent()
        .map(|dir| dir.join("formatters"))
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

/// The formatters, built-in and the user's, and what was wrong with the
/// user's file if anything.
pub struct Registry {
    pub formatters: Vec<Arc<Formatter>>,
    pub error: Option<String>,
}

impl Registry {
    /// The first installed formatter for `language`, for `file`.
    pub fn for_language(&self, language: &str, file: &Path) -> Option<Arc<Formatter>> {
        self.serving(language)
            .find(|f| f.program(file).is_some())
            .cloned()
    }

    /// Every formatter for `language`, installed or not, first first.
    pub fn serving(&self, language: &str) -> impl Iterator<Item = &Arc<Formatter>> {
        self.formatters
            .iter()
            .filter(move |f| f.languages.iter().any(|l| l == language))
    }

    /// A formatter by key.
    pub fn get(&self, key: &str) -> Option<Arc<Formatter>> {
        self.formatters.iter().find(|f| f.key == key).cloned()
    }

    /// Whether any formatter, installed or not, is for `language`.
    pub fn knows(&self, language: &str) -> Option<Arc<Formatter>> {
        self.formatters
            .iter()
            .find(|f| f.languages.iter().any(|l| l == language))
            .cloned()
    }
}

/// The formatters as they are now; the user's file is read again when it
/// has changed.
pub fn registry() -> Arc<Registry> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(registry) = TEST_REGISTRY.with(|test| test.borrow().clone()) {
        return registry;
    }
    type Cached = (Option<SystemTime>, Arc<Registry>);
    static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
    let path = user_path();
    let modified = path
        .as_deref()
        .and_then(|path| std::fs::metadata(path).ok())
        .and_then(|meta| meta.modified().ok());
    let mut cache = CACHE.lock().unwrap();
    if let Some((stamp, registry)) = cache.as_ref()
        && *stamp == modified
    {
        return registry.clone();
    }
    let registry = Arc::new(load(path.as_deref()));
    *cache = Some((modified, registry.clone()));
    registry
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    static TEST_REGISTRY: std::cell::RefCell<Option<Arc<Registry>>> =
        const { std::cell::RefCell::new(None) };
}

/// Use only the formatters in `toml` on this thread, for tests: none of
/// the built-ins, which may well be installed.
#[cfg(any(test, feature = "testing"))]
pub fn use_for_tests(toml: &str) {
    let registry = Registry {
        formatters: parse(toml).unwrap().into_iter().map(Arc::new).collect(),
        error: None,
    };
    TEST_REGISTRY.with(|test| *test.borrow_mut() = Some(Arc::new(registry)));
}

/// The user's own `formatters.toml`; `None` in tests.
pub fn user_path() -> Option<PathBuf> {
    jig_tools::user_file("formatters.toml")
}

fn parse(text: &str) -> Result<Vec<Formatter>> {
    let file: FormatterFile = toml::from_str(text)?;
    for formatter in &file.formatter {
        if formatter.command.is_empty() {
            bail!("formatter “{}” needs a `command`", formatter.key);
        }
    }
    Ok(file.formatter)
}

fn load(user: Option<&Path>) -> Registry {
    let mut formatters = parse(BUILT_IN).expect("the built-in formatters parse");
    let mut error = None;
    if let Some(path) = user
        && let Ok(text) = std::fs::read_to_string(path)
    {
        match parse(&text) {
            Ok(own) => {
                // The user's come first, so one of theirs for a language
                // wins over a built-in for it.
                let mut merged = Vec::new();
                for formatter in own {
                    formatters.retain(|f| f.key != formatter.key);
                    merged.push(formatter);
                }
                merged.append(&mut formatters);
                formatters = merged;
            }
            Err(e) => error = Some(format!("formatters.toml isn't valid: {e:#}")),
        }
    }
    Registry {
        formatters: formatters.into_iter().map(Arc::new).collect(),
        error,
    }
}

/// `text` with trailing spaces and tabs taken off each line, its line
/// endings made `end_of_line`, and ending in a newline if it has any text,
/// as asked.
pub fn tidy(
    text: &str,
    trim_trailing_whitespace: bool,
    final_newline: bool,
    end_of_line: Option<LineEnding>,
) -> String {
    let mut out = if trim_trailing_whitespace {
        let mut out = String::with_capacity(text.len());
        for line in text.split_inclusive('\n') {
            let body = line.trim_end_matches(['\n', '\r']);
            out.push_str(body.trim_end_matches([' ', '\t']));
            out.push_str(&line[body.len()..]);
        }
        out
    } else {
        text.to_string()
    };
    if let Some(ending) = end_of_line {
        out = out.replace("\r\n", "\n");
        if ending == LineEnding::CrLf {
            out = out.replace('\n', "\r\n");
        }
    }
    if final_newline && !out.is_empty() && !out.ends_with('\n') {
        let ending = end_of_line.unwrap_or(if text.contains("\r\n") {
            LineEnding::CrLf
        } else {
            LineEnding::Lf
        });
        out.push_str(ending.as_str());
    }
    out
}

#[cfg(test)]
mod tests;
