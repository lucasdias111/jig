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

use anyhow::{Result, bail};
use serde::Deserialize;

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
}

impl Formatter {
    /// The program to run, if it's installed: on the `PATH`, or in a
    /// `node_modules/.bin` above `file`, where Prettier usually is.
    pub fn program(&self, file: &Path) -> Option<PathBuf> {
        let name = self.command.first()?;
        if name.contains('/') || name.contains('\\') {
            return Some(PathBuf::from(name)).filter(|path| path.is_file());
        }
        let local = file
            .ancestors()
            .skip(1)
            .map(|dir| dir.join("node_modules/.bin"));
        std::env::split_paths(crate::lsp::search_path())
            .chain(local)
            .find_map(|dir| executable_in(&dir, name))
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
            .args(self.args(file, root))
            .env("PATH", crate::lsp::search_path());
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

/// `name`, as an executable in `dir`. Windows adds an extension.
fn executable_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let extensions: &[&str] = if cfg!(windows) {
        &["", ".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    extensions
        .iter()
        .map(|extension| dir.join(format!("{name}{extension}")))
        .find(|candidate| candidate.is_file())
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
        self.formatters
            .iter()
            .filter(|f| f.languages.iter().any(|l| l == language))
            .find(|f| f.program(file).is_some())
            .cloned()
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
    #[cfg(test)]
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

#[cfg(test)]
thread_local! {
    static TEST_REGISTRY: std::cell::RefCell<Option<Arc<Registry>>> =
        const { std::cell::RefCell::new(None) };
}

/// Use only the formatters in `toml` on this thread, for tests: none of
/// the built-ins, which may well be installed.
#[cfg(test)]
pub fn use_for_tests(toml: &str) {
    let registry = Registry {
        formatters: parse(toml).unwrap().into_iter().map(Arc::new).collect(),
        error: None,
    };
    TEST_REGISTRY.with(|test| *test.borrow_mut() = Some(Arc::new(registry)));
}

/// The user's `formatters.toml`; `None` in tests.
pub fn user_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("jig").join("formatters.toml"))
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

/// `text` with trailing spaces and tabs taken off each line, and ending in
/// a newline if it has any text, per the two settings.
pub fn tidy(text: &str, trim_trailing_whitespace: bool, final_newline: bool) -> String {
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
    if final_newline && !out.is_empty() && !out.ends_with('\n') {
        out.push_str(if text.contains("\r\n") { "\r\n" } else { "\n" });
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    const LIMIT: Duration = Duration::from_secs(10);

    #[test]
    fn the_built_ins_parse() {
        let registry = load(None);
        assert!(registry.error.is_none());
        let python: Vec<&str> = registry
            .formatters
            .iter()
            .filter(|f| f.languages.iter().any(|l| l == "python"))
            .map(|f| f.key.as_str())
            .collect();
        assert_eq!(python, ["ruff", "black"], "Ruff first");
        assert!(registry.knows("rust").is_some());
        assert!(registry.knows("markdown").is_some());
    }

    #[test]
    fn the_users_file_replaces_adds_and_comes_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("formatters.toml");
        fs::write(
            &path,
            r#"
[[formatter]]
key = "rustfmt"
name = "My rustfmt"
languages = ["rust"]
command = ["rustfmt", "--edition", "2024"]

[[formatter]]
key = "yapf"
name = "YAPF"
languages = ["python"]
command = ["yapf"]
"#,
        )
        .unwrap();
        let registry = load(Some(&path));
        assert!(registry.error.is_none(), "{:?}", registry.error);
        assert_eq!(registry.knows("rust").unwrap().name, "My rustfmt");
        assert_eq!(registry.knows("python").unwrap().key, "yapf");
        assert_eq!(registry.formatters.len(), load(None).formatters.len() + 1);

        fs::write(
            &path,
            "[[formatter]]\nkey = \"x\"\nname = \"X\"\nlanguages = []\ncommand = []\n",
        )
        .unwrap();
        let broken = load(Some(&path));
        assert!(broken.error.unwrap().contains("command"));
        assert_eq!(broken.formatters.len(), load(None).formatters.len());
    }

    #[test]
    fn formatters_that_arent_installed_are_skipped() {
        // Cargo is running these tests, so it's installed.
        let registry = Registry {
            formatters: parse(
                r#"
[[formatter]]
key = "missing"
name = "Missing"
languages = ["rust"]
command = ["jig-no-such-formatter"]

[[formatter]]
key = "cargo"
name = "Cargo"
languages = ["rust"]
command = ["cargo", "fmt"]
"#,
            )
            .unwrap()
            .into_iter()
            .map(Arc::new)
            .collect(),
            error: None,
        };
        let file = Path::new("/p/src/main.rs");
        assert_eq!(registry.for_language("rust", file).unwrap().key, "cargo");
        assert!(registry.for_language("go", file).is_none());
        let missing = registry.knows("rust").unwrap();
        assert_eq!(
            missing.format("x", file, Path::new("/p"), LIMIT),
            Err("Missing isn't installed.".into())
        );
    }

    fn fake(script: &str) -> Formatter {
        Formatter {
            key: "fake".into(),
            name: "Fake".into(),
            languages: vec!["rust".into()],
            command: vec!["/bin/sh".into(), "-c".into(), script.into()],
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_fake_formatter_gets_stdin_and_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.rs");
        let formatter = Formatter {
            command: vec![
                "/bin/sh".into(),
                "-c".into(),
                r#"tr a-z A-Z; printf '%s\n' "$1""#.into(),
                "sh".into(),
                "${file}".into(),
            ],
            ..fake("")
        };
        let out = formatter.format("fn a() {}\n", &file, dir.path(), LIMIT);
        assert_eq!(out.unwrap(), format!("FN A() {{}}\n{}\n", file.display()));
        // A Windows file keeps its line endings.
        let out = fake("cat").format("a\r\nb\r\n", &file, dir.path(), LIMIT);
        assert_eq!(out.unwrap(), "a\r\nb\r\n");
        let out = fake("tr -d '\\r'").format("a\r\nb\r\n", &file, dir.path(), LIMIT);
        assert_eq!(out.unwrap(), "a\r\nb\r\n");
    }

    #[cfg(unix)]
    #[test]
    fn failures_say_why_and_never_empty_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.rs");
        let failing = fake("cat >/dev/null; echo 'error: expected `;`' >&2; exit 1");
        assert_eq!(
            failing.format("x", &file, dir.path(), LIMIT),
            Err("Fake: error: expected `;`".into())
        );
        let silent = fake("cat >/dev/null");
        assert_eq!(
            silent.format("x", &file, dir.path(), LIMIT),
            Err("Fake: returned nothing".into())
        );
        let slow = fake("sleep 5");
        let started = Instant::now();
        let out = slow.format("x", &file, dir.path(), Duration::from_millis(200));
        assert!(out.unwrap_err().contains("no answer"));
        assert!(started.elapsed() < Duration::from_secs(3), "stopped");
    }

    #[test]
    fn tidy_trims_and_ends_with_a_newline() {
        let text = "a  \n\tb\t\r\n  \nc ";
        assert_eq!(tidy(text, true, false), "a\n\tb\r\n\nc");
        assert_eq!(tidy(text, true, true), "a\n\tb\r\n\nc\r\n");
        assert_eq!(tidy(text, false, true), "a  \n\tb\t\r\n  \nc \r\n");
        assert_eq!(tidy("x\n", false, true), "x\n");
        assert_eq!(tidy("", true, true), "", "an empty file stays empty");
        assert_eq!(tidy(text, false, false), text);
    }
}
