//! Running a configuration: the process, and its output made into lines to
//! show, with terminal colours kept and `file:line` references found.
//!
//! The process gets no terminal, so programs that check for one don't
//! colour their output on their own; the environment asks them to. It runs
//! in its own process group (on Windows, its own process tree), so stopping
//! it stops whatever it started, such as the program `cargo run` builds.

use std::io::{BufRead as _, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::{Context as _, Result};
use futures::channel::mpsc::UnboundedSender;
#[cfg(windows)]
use jig_process::NoWindow as _;

use crate::domain::output::{self, OutputLine, RunEvent, Stream, Style};
use crate::domain::run_configs::RunConfig;

/// How long a stopped process gets to exit before it is killed.
#[cfg(unix)]
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

/// A running configuration. Dropping it stops the process.
pub struct Process {
    pid: i32,
    exited: Arc<AtomicBool>,
    /// Set by `stop`, so the ending says so: on Windows the process just
    /// ends with whatever code it's killed with.
    stopped: Arc<AtomicBool>,
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
        let mut command = shell_command(&config.command);
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let mut child = command
            .current_dir(&config.cwd)
            .env("PATH", jig_tools::search_path())
            .envs(COLOR_ENV.iter().copied())
            .envs(config.env.iter().map(|(k, v)| (k, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("Couldn't start “{}”", config.command))?;
        let pid = child.id() as i32;
        let exited = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
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

        let (flag, was_stopped) = (exited.clone(), stopped.clone());
        std::thread::spawn(move || {
            let status = child.wait();
            flag.store(true, Ordering::SeqCst);
            for _ in 0..2 {
                if drained.recv_timeout(DRAIN_TIMEOUT).is_err() {
                    break;
                }
            }
            let (message, success) = match status {
                Ok(status) if !status.success() && was_stopped.load(Ordering::SeqCst) => {
                    ("Process stopped".into(), false)
                }
                Ok(status) => (describe_exit(status), status.success()),
                Err(error) => (format!("Lost track of the process: {error}"), false),
            };
            let _ = events.unbounded_send(RunEvent::Exited { message, success });
        });
        Ok(Self {
            pid,
            exited,
            stopped,
        })
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
        self.stopped.store(true, Ordering::SeqCst);
        #[cfg(unix)]
        {
            signal_group(self.pid, libc::SIGTERM);
            let (pid, exited) = (self.pid, self.exited.clone());
            std::thread::spawn(move || {
                std::thread::sleep(STOP_GRACE);
                if !exited.load(Ordering::SeqCst) {
                    signal_group(pid, libc::SIGKILL);
                }
            });
        }
        // Windows has no polite stop for a console program without a window
        // of its own, so the tree goes at once.
        #[cfg(not(unix))]
        let _ = Command::new("taskkill")
            .no_window()
            .args(["/T", "/F", "/PID", &self.pid.to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.stop();
    }
}

/// A command running `script` through the system shell: `/bin/sh`, or
/// `cmd` on Windows.
pub fn shell_command(script: &str) -> Command {
    #[cfg(unix)]
    {
        let mut command = Command::new("/bin/sh");
        command.arg("-c").arg(script);
        command
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        let mut command = Command::new("cmd");
        // cmd parses its own command line, so the script goes as written.
        command.no_window().arg("/C").raw_arg(script);
        command
    }
}

#[cfg(unix)]
fn signal_group(pid: i32, signal: libc::c_int) {
    // SAFETY: plain syscall; the group is the child's own, made at spawn.
    unsafe {
        libc::killpg(pid, signal);
    }
}

#[cfg(unix)]
fn describe_exit(status: ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt as _;
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("Process finished with exit code {code}"),
        (None, Some(libc::SIGTERM | libc::SIGKILL)) => "Process stopped".into(),
        (None, Some(signal)) => format!("Process ended by signal {signal}"),
        (None, None) => "Process finished".into(),
    }
}

#[cfg(not(unix))]
fn describe_exit(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("Process finished with exit code {code}"),
        None => "Process finished".into(),
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

/// One line of output as shown, its file references resolved against the
/// first of `bases` they're found in on disk.
pub fn parse_line(raw: &str, stream: Stream, style: &mut Style, bases: &[PathBuf]) -> OutputLine {
    output::parse_line(raw, stream, style, bases, Path::is_file)
}
