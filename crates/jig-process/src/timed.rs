//! Running another program to the end, but not forever: installers and
//! git's network commands can hang on a stalled download or a remote that
//! never answers, and nothing would ever tell the user.

use std::io::{self, Read};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Run `command` with its output collected, as [`Command::output`] does,
/// but stop it after `limit` with an [`io::ErrorKind::TimedOut`] error.
/// Its stdin is left as the command set it.
///
/// On Unix, a command started in a process group of its own (or a session
/// of its own) is stopped with everything it started, so an installer's
/// `npm` or git's `ssh` doesn't carry on behind it.
pub fn output_within(command: &mut Command, limit: Duration) -> io::Result<Output> {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Read on threads of their own, so a chatty program can't block on a
    // full pipe while this waits for it.
    let read = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut bytes);
            }
            bytes
        })
    };
    let stdout = read(child.stdout.take().map(|pipe| Box::new(pipe) as _));
    let stderr = read(child.stderr.take().map(|pipe| Box::new(pipe) as _));
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= limit {
            stop(&mut child);
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("stopped after {}", describe(limit)),
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    Ok(Output {
        status,
        stdout: stdout.join().unwrap_or_default(),
        stderr: stderr.join().unwrap_or_default(),
    })
}

/// `limit` as people say it: "90 s", "5 minutes".
pub fn describe(limit: Duration) -> String {
    let seconds = limit.as_secs();
    if seconds >= 120 && seconds.is_multiple_of(60) {
        format!("{} minutes", seconds / 60)
    } else {
        format!("{seconds} s")
    }
}

fn stop(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        // SAFETY: plain syscall. Only a group the child leads has its pid
        // as its id; without one this finds nothing and the kill below
        // still stops the child.
        unsafe {
            libc::killpg(pid, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        use crate::NoWindow as _;
        let _ = Command::new("taskkill")
            .no_window()
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn output_is_collected_as_usual() {
        let output = output_within(
            Command::new("/bin/sh").args(["-c", "echo out; echo err >&2; exit 3"]),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(output.stdout, b"out\n");
        assert_eq!(output.stderr, b"err\n");
        assert_eq!(output.status.code(), Some(3));
    }

    #[test]
    fn a_program_past_its_time_is_stopped_with_what_it_started() {
        let mut command = Command::new("/bin/sh");
        // The shell's child holds the pipes open; stopping only the shell
        // would leave this waiting on it.
        command.args(["-c", "sleep 30; true"]);
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        let started = Instant::now();
        let error = output_within(&mut command, Duration::from_millis(200)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn limits_read_naturally() {
        assert_eq!(describe(Duration::from_secs(90)), "90 s");
        assert_eq!(describe(Duration::from_secs(600)), "10 minutes");
    }
}
