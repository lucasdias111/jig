//! Running a configuration: the process, and its output made into lines to
//! show, with terminal colours kept and `file:line` references found.
//!
//! The process gets no terminal, so programs that check for one don't
//! colour their output on their own; the environment asks them to. It runs
//! in its own process group, so stopping it stops whatever it started, such
//! as the program `cargo run` builds.

use std::io::{BufRead as _, BufReader, Read};
use std::ops::Range;
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::channel::mpsc::UnboundedSender;
use gpui_kit::SharedString;
use regex::Regex;

use crate::run_configs::RunConfig;

/// Longer lines are cut, so one huge line can't stall layout.
const MAX_LINE_CHARS: usize = 4000;
/// How long a stopped process gets to exit before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(3);
/// How long output still arriving after the process exited is waited for,
/// in case something it started holds the pipes open.
const DRAIN_TIMEOUT: Duration = Duration::from_millis(500);

/// Ask tools that colour only for terminals to colour anyway.
const COLOR_ENV: &[(&str, &str)] = &[
    ("CARGO_TERM_COLOR", "always"),
    ("CLICOLOR_FORCE", "1"),
    ("FORCE_COLOR", "1"),
    ("PY_COLORS", "1"),
];

pub enum RunEvent {
    Line(OutputLine),
    /// A line of JSON on stdout, kept raw for the caller to read, when the
    /// process was started for its data (`cargo build --message-format`).
    Data(String),
    /// How it ended, in words, and whether it succeeded.
    Exited {
        message: String,
        success: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
    /// Jig's own lines: the command, and how it ended.
    Meta,
}

#[derive(Clone, Debug, PartialEq)]
pub struct OutputLine {
    pub text: SharedString,
    pub stream: Stream,
    /// Styled ranges of `text`; the rest is unstyled.
    pub spans: Vec<(Range<usize>, Style)>,
    pub links: Vec<Link>,
}

impl OutputLine {
    pub fn meta(text: impl Into<SharedString>) -> Self {
        Self {
            text: text.into(),
            stream: Stream::Meta,
            spans: Vec::new(),
            links: Vec::new(),
        }
    }
}

/// A file mentioned in the output, which clicking opens.
#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub range: Range<usize>,
    pub path: PathBuf,
    /// 1-based.
    pub line: u32,
    /// 1-based.
    pub column: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Style {
    pub fg: Option<Color>,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// One of the 16 terminal colours.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// A running configuration. Dropping it stops the process.
pub struct Process {
    pid: i32,
    exited: Arc<AtomicBool>,
}

impl Process {
    /// Start `config`, sending its output and then its end to `events`.
    /// File references resolve against its folder, then `root`. With
    /// `json_stdout`, lines of JSON on stdout come as data, not output.
    pub fn spawn(
        config: &RunConfig,
        root: &Path,
        json_stdout: bool,
        events: UnboundedSender<RunEvent>,
    ) -> Result<Self> {
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(&config.command)
            .current_dir(&config.cwd)
            .env("PATH", crate::lsp::search_path())
            .envs(COLOR_ENV.iter().copied())
            .envs(config.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .with_context(|| format!("Couldn't start “{}”", config.command))?;
        let pid = child.id() as i32;
        let exited = Arc::new(AtomicBool::new(false));
        let bases = Arc::new([config.cwd.clone(), root.to_path_buf()]);

        let (done, drained) = mpsc::channel();
        let streams = [
            child
                .stdout
                .take()
                .map(|out| (Box::new(out) as Box<dyn Read + Send>, Stream::Stdout)),
            child
                .stderr
                .take()
                .map(|err| (Box::new(err) as Box<dyn Read + Send>, Stream::Stderr)),
        ];
        for (reader, stream) in streams.into_iter().flatten() {
            let events = events.clone();
            let bases = bases.clone();
            let done = done.clone();
            std::thread::spawn(move || {
                let json = json_stdout && stream == Stream::Stdout;
                read_lines(reader, stream, json, &bases[..], &events);
                let _ = done.send(());
            });
        }
        drop(done);

        let flag = exited.clone();
        std::thread::spawn(move || {
            let status = child.wait();
            flag.store(true, Ordering::SeqCst);
            for _ in 0..2 {
                if drained.recv_timeout(DRAIN_TIMEOUT).is_err() {
                    break;
                }
            }
            let (message, success) = match status {
                Ok(status) => (describe_exit(status), status.success()),
                Err(error) => (format!("Lost track of the process: {error}"), false),
            };
            let _ = events.unbounded_send(RunEvent::Exited { message, success });
        });
        Ok(Self { pid, exited })
    }

    pub fn is_running(&self) -> bool {
        !self.exited.load(Ordering::SeqCst)
    }

    /// Ask the process and everything it started to stop, and kill them if
    /// they're still there shortly after.
    pub fn stop(&self) {
        if !self.is_running() {
            return;
        }
        signal_group(self.pid, libc::SIGTERM);
        let (pid, exited) = (self.pid, self.exited.clone());
        std::thread::spawn(move || {
            std::thread::sleep(STOP_GRACE);
            if !exited.load(Ordering::SeqCst) {
                signal_group(pid, libc::SIGKILL);
            }
        });
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

fn signal_group(pid: i32, signal: libc::c_int) {
    // SAFETY: plain syscall; the group is the child's own, made at spawn.
    unsafe {
        libc::killpg(pid, signal);
    }
}

fn describe_exit(status: ExitStatus) -> String {
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("Process finished with exit code {code}"),
        (None, Some(libc::SIGTERM | libc::SIGKILL)) => "Process stopped".into(),
        (None, Some(signal)) => format!("Process ended by signal {signal}"),
        (None, None) => "Process finished".into(),
    }
}

fn read_lines(
    reader: impl Read,
    stream: Stream,
    json: bool,
    bases: &[PathBuf],
    events: &UnboundedSender<RunEvent>,
) {
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    let mut style = Style::default();
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let raw = String::from_utf8_lossy(&buf);
        let event = if json && raw.starts_with('{') {
            RunEvent::Data(raw.trim_end().to_string())
        } else {
            RunEvent::Line(parse_line(&raw, stream, &mut style, bases))
        };
        if events.unbounded_send(event).is_err() {
            return;
        }
    }
}

/// One line of output as shown. `style` carries colour from line to line,
/// as a terminal does; references resolve against the first of `bases`
/// they're found in.
pub fn parse_line(raw: &str, stream: Stream, style: &mut Style, bases: &[PathBuf]) -> OutputLine {
    let raw = raw.trim_end_matches(['\n', '\r']);
    // A carriage return redraws the line, as progress bars do: keep the last.
    let raw = raw.rsplit('\r').next().unwrap_or(raw);
    let (text, spans) = strip_ansi(raw, style);
    let links = find_links(&text, bases);
    OutputLine {
        text: text.into(),
        stream,
        spans,
        links,
    }
}

/// `raw` without escape sequences, and the ranges its colour codes styled.
pub fn strip_ansi(raw: &str, style: &mut Style) -> (String, Vec<(Range<usize>, Style)>) {
    let mut text = String::with_capacity(raw.len());
    let mut spans: Vec<(Range<usize>, Style)> = Vec::new();
    let mut chars = raw.chars().peekable();
    let mut count = 0;
    while let Some(c) = chars.next() {
        match c {
            '\x1b' => match chars.next() {
                Some('[') => {
                    let mut params = String::new();
                    for c in chars.by_ref() {
                        if ('\x40'..='\x7e').contains(&c) {
                            if c == 'm' {
                                apply_sgr(&params, style);
                            }
                            break;
                        }
                        params.push(c);
                    }
                }
                // Operating system commands, e.g. hyperlinks: up to BEL or ESC \.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\x07' || (c == '\x1b' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\t' => {
                let spaces = 4 - count % 4;
                push(&mut text, &mut spans, &" ".repeat(spaces), *style);
                count += spaces;
            }
            c if c.is_control() => {}
            c => {
                if count >= MAX_LINE_CHARS {
                    text.push('…');
                    break;
                }
                push(&mut text, &mut spans, c.encode_utf8(&mut [0; 4]), *style);
                count += 1;
            }
        }
    }
    (text, spans)
}

fn push(text: &mut String, spans: &mut Vec<(Range<usize>, Style)>, s: &str, style: Style) {
    let start = text.len();
    text.push_str(s);
    if style == Style::default() {
        return;
    }
    match spans.last_mut() {
        Some((range, last)) if *last == style && range.end == start => range.end = text.len(),
        _ => spans.push((start..text.len(), style)),
    }
}

/// Apply a Select Graphic Rendition sequence's parameters to `style`.
fn apply_sgr(params: &str, style: &mut Style) {
    let codes: Vec<u16> = params
        .split([';', ':'])
        .map(|p| p.parse().unwrap_or(0))
        .collect();
    let mut codes = codes.into_iter();
    while let Some(code) = codes.next() {
        match code {
            0 => *style = Style::default(),
            1 => style.bold = true,
            2 => style.dim = true,
            3 => style.italic = true,
            4 => style.underline = true,
            22 => (style.bold, style.dim) = (false, false),
            23 => style.italic = false,
            24 => style.underline = false,
            30..=37 => style.fg = Some(Color::Indexed((code - 30) as u8)),
            90..=97 => style.fg = Some(Color::Indexed((code - 90 + 8) as u8)),
            39 => style.fg = None,
            38 | 48 => {
                let color = match codes.next() {
                    Some(5) => codes.next().map(|n| match n {
                        0..=15 => Color::Indexed(n as u8),
                        n => indexed_rgb(n as u8),
                    }),
                    Some(2) => {
                        let mut channel = || codes.next().unwrap_or(0).min(255) as u8;
                        Some(Color::Rgb(channel(), channel(), channel()))
                    }
                    _ => None,
                };
                if code == 38 {
                    style.fg = color;
                }
            }
            _ => {}
        }
    }
}

/// A colour of the 256-colour palette past the first 16.
fn indexed_rgb(n: u8) -> Color {
    if n >= 232 {
        let level = 8 + (n - 232) * 10;
        return Color::Rgb(level, level, level);
    }
    let n = n - 16;
    let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
    Color::Rgb(level(n / 36), level(n / 6 % 6), level(n % 6))
}

fn path_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    // `src/main.rs:12:5`, `/abs/file.ts:3`, `./a.py:9`; a file needs an
    // extension, so times like `12:30:01` don't match.
    PATTERN.get_or_init(|| {
        Regex::new(
            r"(?:^|[^\w./@+-])((?:\.{0,2}/)?(?:[\w.@+-]+/)*[\w@+-][\w.@+-]*\.\w+):(\d+)(?::(\d+))?",
        )
        .unwrap()
    })
}

fn python_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r#"File "([^"]+)", line (\d+)"#).unwrap())
}

/// References in `text` to files that exist.
pub fn find_links(text: &str, bases: &[PathBuf]) -> Vec<Link> {
    if !text.contains(':') && !text.contains("line ") {
        return Vec::new();
    }
    let mut links = Vec::new();
    for pattern in [path_pattern(), python_pattern()] {
        for captures in pattern.captures_iter(text) {
            let (Some(file), Some(line)) = (captures.get(1), captures.get(2)) else {
                continue;
            };
            let Some(path) = resolve(file.as_str(), bases) else {
                continue;
            };
            let column = captures.get(3);
            let end = column.map_or(line.end(), |c| c.end());
            links.push(Link {
                range: file.start()..end,
                path,
                line: line.as_str().parse().unwrap_or(1).max(1),
                column: column.and_then(|c| c.as_str().parse().ok()),
            });
        }
    }
    links.sort_by_key(|link| link.range.start);
    links
}

fn resolve(file: &str, bases: &[PathBuf]) -> Option<PathBuf> {
    let path = Path::new(file);
    if path.is_absolute() {
        return path.is_file().then(|| path.to_path_buf());
    }
    bases
        .iter()
        .map(|base| base.join(path))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn red() -> Style {
        Style {
            fg: Some(Color::Indexed(1)),
            ..Default::default()
        }
    }

    #[test]
    fn colours_become_spans_and_escapes_go() {
        let mut style = Style::default();
        let (text, spans) = strip_ansi("\x1b[1m\x1b[31merror\x1b[0m: oops\x1b[K", &mut style);
        assert_eq!(text, "error: oops");
        assert_eq!(
            spans,
            [(
                0..5,
                Style {
                    bold: true,
                    ..red()
                }
            )]
        );
        assert_eq!(style, Style::default());
    }

    #[test]
    fn colour_carries_over_to_the_next_line() {
        let mut style = Style::default();
        strip_ansi("\x1b[31mstart", &mut style);
        let (_, spans) = strip_ansi("more", &mut style);
        assert_eq!(spans, [(0..4, red())]);
    }

    #[test]
    fn extended_colours_and_hyperlinks() {
        let mut style = Style::default();
        let (text, spans) = strip_ansi(
            "\x1b]8;;file:///x\x1b\\link\x1b]8;;\x1b\\ \x1b[38;2;10;20;30mrgb\x1b[38;5;9m!",
            &mut style,
        );
        assert_eq!(text, "link rgb!");
        assert_eq!(spans[0].1.fg, Some(Color::Rgb(10, 20, 30)));
        assert_eq!(
            spans[1],
            (
                8..9,
                Style {
                    fg: Some(Color::Indexed(9)),
                    ..Default::default()
                }
            )
        );
    }

    #[test]
    fn carriage_returns_keep_the_last_redraw() {
        let line = parse_line(
            "10%\r50%\r100%\r\n",
            Stream::Stdout,
            &mut Style::default(),
            &[],
        );
        assert_eq!(line.text.as_ref(), "100%");
    }

    #[test]
    fn existing_files_are_linked_with_line_and_column() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "").unwrap();
        fs::write(root.join("app.py"), "").unwrap();
        let bases = [root.join("sub"), root.clone()];

        let links = find_links("  --> src/main.rs:12:5", &bases);
        assert_eq!(
            links,
            [Link {
                range: 6..22,
                path: root.join("src/main.rs"),
                line: 12,
                column: Some(5),
            }]
        );
        let links = find_links(r#"  File "app.py", line 3, in <module>"#, &bases);
        assert_eq!(links[0].path, root.join("app.py"));
        assert_eq!(links[0].line, 3);

        assert!(find_links("src/missing.rs:1:1 at 12:30:01", &bases).is_empty());
    }

    #[test]
    fn runs_stops_and_reports_the_ending() {
        let dir = tempfile::tempdir().unwrap();
        let config = RunConfig {
            name: "t".into(),
            command: "echo out; echo err >&2; exit 3".into(),
            cwd: dir.path().to_path_buf(),
            env: Vec::new(),
            source: crate::run_configs::Source::File,
            debug: None,
            debugger: None,
        };
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let process = Process::spawn(&config, dir.path(), false, tx).unwrap();
        let events: Vec<RunEvent> = futures::executor::block_on_stream(rx).collect();
        let mut lines: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                RunEvent::Line(line) => Some((line.text.to_string(), line.stream)),
                _ => None,
            })
            .collect();
        lines.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            lines,
            [
                ("err".into(), Stream::Stderr),
                ("out".into(), Stream::Stdout)
            ]
        );
        assert!(matches!(
            events.last(),
            Some(RunEvent::Exited { message, success: false })
                if message == "Process finished with exit code 3"
        ));
        assert!(!process.is_running());

        let config = RunConfig {
            command: "sleep 30".into(),
            ..config
        };
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let process = Process::spawn(&config, dir.path(), false, tx).unwrap();
        process.stop();
        let ending = futures::executor::block_on_stream(rx).find_map(|e| match e {
            RunEvent::Exited { message, .. } => Some(message),
            _ => None,
        });
        assert_eq!(ending.as_deref(), Some("Process stopped"));
    }
}
