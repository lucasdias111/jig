//! Git, through the `git` command line rather than a library, so the user's
//! own config, hooks, credential helpers and SSH agent all apply. Every call
//! blocks; the workspace runs them in the background.
//!
//! Git never gets a terminal to ask on: a push that needs a password fails
//! with git's message instead of waiting for an answer nobody can give.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};
use jig_process::NoWindow as _;

use crate::domain::status::{
    Branch, MAX_NEW_FOLDER_FILES, Status, explain, parse_branches, parse_status,
};

/// How long fetch, pull and push get. A remote that stops answering would
/// otherwise leave the Git view spinning for good.
const NETWORK_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// A working tree, by its top folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    root: PathBuf,
}

impl Repo {
    /// The repository `path` (a file or folder) is in, if any.
    pub fn discover(path: &Path) -> Option<Repo> {
        let dir = if path.is_dir() { path } else { path.parent()? };
        let output = command(dir)
            .args(["rev-parse", "--show-toplevel"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let root = String::from_utf8(output.stdout).ok()?;
        let root = PathBuf::from(root.trim_end_matches(['\n', '\r']));
        Some(Repo {
            root: root.canonicalize().unwrap_or(root),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `path` as git names it: relative to the root, `/` between folders.
    pub fn relative(&self, path: &Path) -> Option<String> {
        let relative = path.strip_prefix(&self.root).ok()?;
        let parts: Vec<_> = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy())
            .collect();
        Some(parts.join("/"))
    }

    /// `path`'s text as last committed, line endings as a checkout would
    /// write them; `None` if it isn't committed or isn't text.
    pub fn committed_text(&self, path: &Path) -> Option<String> {
        let relative = self.relative(path)?;
        let output = self
            .git(&["cat-file", "--filters", &format!("HEAD:{relative}")])
            .ok()?;
        String::from_utf8(output.stdout).ok()
    }

    pub fn status(&self) -> Result<Status> {
        let output = self.git(&[
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--branch",
            // Untracked folders as one entry each, as `git status` shows
            // them: a build folder can hold thousands of files.
            "--untracked-files=normal",
            "-z",
        ])?;
        Ok(parse_status(&String::from_utf8_lossy(&output.stdout)))
    }

    /// Stage `paths` as they are now, deleted ones included. An untracked
    /// folder of more than [`MAX_NEW_FOLDER_FILES`] files is refused: that's
    /// build output, which belongs in `.gitignore`, not a commit.
    pub fn stage(&self, paths: &[String]) -> Result<()> {
        for folder in paths.iter().filter(|path| path.ends_with('/')) {
            let count = self.new_files_in(folder)?;
            if count > MAX_NEW_FOLDER_FILES {
                bail!(
                    "{folder} holds {count} new files, which looks like build output. \
                     Leave it unchecked, or add it to .gitignore."
                );
            }
        }
        self.git_on_paths(&["add", "--all"], paths).map(drop)
    }

    /// How many untracked, unignored files `folder` holds.
    fn new_files_in(&self, folder: &str) -> Result<usize> {
        let output = self.git(&[
            "ls-files",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            folder,
        ])?;
        Ok(output.stdout.iter().filter(|&&byte| byte == 0).count())
    }

    pub fn unstage(&self, paths: &[String]) -> Result<()> {
        // Before the first commit there's nothing to restore from.
        let args: &[&str] = if self.has_commits() {
            &["restore", "--staged"]
        } else {
            &["rm", "--cached", "-r", "-q"]
        };
        self.git_on_paths(args, paths).map(drop)
    }

    /// Commit `paths` as they are now, and nothing else: changes staged in
    /// other files stay staged.
    pub fn commit(&self, message: &str, paths: &[String]) -> Result<()> {
        if message.trim().is_empty() {
            bail!("Write a commit message first.");
        }
        if paths.is_empty() {
            bail!("Check the files to commit.");
        }
        // New files have to be known to git before a commit can name them.
        self.stage(paths)?;
        self.git_on_paths(&["commit", "--quiet", "--only", "-m", message], paths)
            .map(drop)
    }

    pub fn fetch(&self) -> Result<()> {
        self.over_network(&["fetch", "--quiet"]).map(drop)
    }

    pub fn pull(&self) -> Result<()> {
        if self.upstream().is_none() {
            bail!("This branch has no upstream to pull from. Push it first.");
        }
        self.over_network(&["pull", "--quiet"]).map(drop)
    }

    /// Push the branch, giving it an upstream on the first push.
    pub fn push(&self) -> Result<()> {
        if self.upstream().is_some() {
            return self.over_network(&["push", "--quiet"]).map(drop);
        }
        let remotes = self.git(&["remote"])?;
        let remotes = String::from_utf8_lossy(&remotes.stdout);
        let remote = remotes
            .lines()
            .find(|remote| *remote == "origin")
            .or_else(|| remotes.lines().next())
            .ok_or_else(|| anyhow!("This repository has no remote to push to."))?;
        self.over_network(&["push", "--quiet", "--set-upstream", remote, "HEAD"])
            .map(drop)
    }

    /// Local branches, then remote ones without a local branch of the same
    /// name, each most recently committed first.
    pub fn branches(&self) -> Result<Vec<Branch>> {
        let output = self.git(&[
            "for-each-ref",
            "--sort=-committerdate",
            "--format=%(HEAD)%09%(refname)",
            "refs/heads",
            "refs/remotes",
        ])?;
        Ok(parse_branches(&String::from_utf8_lossy(&output.stdout)))
    }

    /// Check out `branch`. A remote branch gets a local one tracking it.
    pub fn switch(&self, branch: &Branch) -> Result<()> {
        if branch.remote {
            self.git(&["switch", "--track", &branch.name]).map(drop)
        } else {
            self.git(&["switch", &branch.name]).map(drop)
        }
    }

    /// Start a branch called `name` here, and check it out.
    pub fn create_branch(&self, name: &str) -> Result<()> {
        self.git(&["switch", "--create", name]).map(drop)
    }

    fn upstream(&self) -> Option<String> {
        let output = self
            .git(&[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ])
            .ok()?;
        Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    fn has_commits(&self) -> bool {
        self.git(&["rev-parse", "--verify", "--quiet", "HEAD"])
            .is_ok()
    }

    /// Run git in the root; a failure carries git's own explanation.
    fn git(&self, args: &[&str]) -> Result<Output> {
        patiently(|| {
            command(&self.root)
                .args(args)
                .output()
                .context("Couldn't run git. Is it installed?")
        })
    }

    /// Run a git command that talks to a remote, stopped after
    /// [`NETWORK_TIMEOUT`]. Git runs in a session of its own, so its `ssh`
    /// stops with it.
    fn over_network(&self, args: &[&str]) -> Result<Output> {
        patiently(|| {
            match jig_process::output_within(command(&self.root).args(args), NETWORK_TIMEOUT) {
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => bail!(
                    "git {} got no answer from the remote in {}, so it was stopped.",
                    args[0],
                    jig_process::describe(NETWORK_TIMEOUT)
                ),
                result => result.context("Couldn't run git. Is it installed?"),
            }
        })
    }

    /// Run git on `paths`, given on its standard input rather than the
    /// command line, which thousands of paths would overflow.
    fn git_on_paths(&self, args: &[&str], paths: &[String]) -> Result<Output> {
        use std::io::Write as _;
        let mut input = paths.join("\0");
        input.push('\0');
        patiently(|| {
            let mut child = command(&self.root)
                .args(args)
                .args(["--pathspec-from-file=-", "--pathspec-file-nul"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .context("Couldn't run git. Is it installed?")?;
            // Git may stop before reading its paths (another git holding the
            // index's lock, say); its exit and stderr then say why, not the
            // broken pipe.
            if let Some(mut stdin) = child.stdin.take()
                && let Err(error) = stdin.write_all(input.as_bytes())
                && error.kind() != std::io::ErrorKind::BrokenPipe
            {
                return Err(error.into());
            }
            Ok(child.wait_with_output()?)
        })
    }
}

/// Run git with `run`, again for up to two seconds while another git (a
/// terminal, another tool) holds the index's lock.
fn patiently(run: impl Fn() -> Result<Output>) -> Result<Output> {
    for _ in 0..20 {
        let output = run()?;
        if output.status.success() || !String::from_utf8_lossy(&output.stderr).contains(".lock'") {
            return checked(output);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    checked(run()?)
}

/// `output`, or git's explanation when it failed.
fn checked(output: Output) -> Result<Output> {
    if output.status.success() {
        return Ok(output);
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    Err(anyhow!(explain(&stderr, &stdout)))
}

pub(crate) fn command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .no_window()
        .arg("-C")
        .arg(dir)
        .args(["-c", "core.quotepath=off", "-c", "color.ui=false"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    // A session of its own has no terminal, so SSH can't stop to ask for a
    // passphrase there either.
    #[cfg(unix)]
    // SAFETY: `setsid` is async-signal-safe, so it may run between fork and
    // exec; it touches nothing of the parent's.
    unsafe {
        use std::os::unix::process::CommandExt as _;
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    command
}
