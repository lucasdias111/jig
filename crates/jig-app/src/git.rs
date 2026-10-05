//! Git, through the `git` command line rather than a library, so the user's
//! own config, hooks, credential helpers and SSH agent all apply. Every call
//! blocks; the workspace runs them in the background.
//!
//! Git never gets a terminal to ask on: a push that needs a password fails
//! with git's message instead of waiting for an answer nobody can give.

use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, bail};

/// Untracked folders with more files than this aren't staged.
pub const MAX_NEW_FOLDER_FILES: usize = 1000;

/// A working tree, by its top folder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repo {
    root: PathBuf,
}

/// What `git status` says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The branch checked out; `None` when HEAD is detached.
    pub branch: Option<String>,
    /// The commit checked out, abbreviated; `None` before the first commit.
    pub commit: Option<String>,
    pub upstream: Option<String>,
    /// Commits not yet pushed to the upstream, and not yet pulled from it.
    pub ahead: u32,
    pub behind: u32,
    pub files: Vec<FileStatus>,
}

/// One changed file. A file can be both staged and changed again since.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileStatus {
    /// Relative to the repository's root, with `/` between folders.
    pub path: String,
    pub staged: Option<Change>,
    pub unstaged: Option<Change>,
    /// For a rename, the path it had.
    pub original: Option<String>,
}

impl FileStatus {
    pub fn is_untracked(&self) -> bool {
        self.unstaged == Some(Change::Untracked)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Conflicted,
}

impl Change {
    /// The letter `git status --short` shows.
    pub fn letter(self) -> &'static str {
        match self {
            Change::Added => "A",
            Change::Modified => "M",
            Change::Deleted => "D",
            Change::Renamed => "R",
            Change::Untracked => "U",
            Change::Conflicted => "!",
        }
    }

    fn from_code(code: char) -> Option<Self> {
        match code {
            'A' | 'C' => Some(Change::Added),
            'M' | 'T' => Some(Change::Modified),
            'D' => Some(Change::Deleted),
            'R' => Some(Change::Renamed),
            'U' => Some(Change::Conflicted),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Branch {
    /// `main`, or for a remote branch, `origin/main`.
    pub name: String,
    pub remote: bool,
    pub current: bool,
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
        self.git(&["fetch", "--quiet"]).map(drop)
    }

    pub fn pull(&self) -> Result<()> {
        if self.upstream().is_none() {
            bail!("This branch has no upstream to pull from. Push it first.");
        }
        self.git(&["pull", "--quiet"]).map(drop)
    }

    /// Push the branch, giving it an upstream on the first push.
    pub fn push(&self) -> Result<()> {
        if self.upstream().is_some() {
            return self.git(&["push", "--quiet"]).map(drop);
        }
        let remotes = self.git(&["remote"])?;
        let remotes = String::from_utf8_lossy(&remotes.stdout);
        let remote = remotes
            .lines()
            .find(|remote| *remote == "origin")
            .or_else(|| remotes.lines().next())
            .ok_or_else(|| anyhow!("This repository has no remote to push to."))?;
        self.git(&["push", "--quiet", "--set-upstream", remote, "HEAD"])
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
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(input.as_bytes())?;
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

fn command(dir: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(dir)
        .args(["-c", "core.quotepath=off", "-c", "color.ui=false"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null());
    // A session of its own has no terminal, so SSH can't stop to ask for a
    // passphrase there either.
    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt as _;
        command.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    command
}

/// Git's message for a failure, without its hints, at most a few lines.
fn explain(stderr: &str, stdout: &str) -> String {
    let text = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("hint:"))
        .map(|line| {
            line.strip_prefix("error: ")
                .or_else(|| line.strip_prefix("fatal: "))
                .unwrap_or(line)
        })
        .take(4)
        .collect();
    if lines.is_empty() {
        "Git failed without saying why.".to_string()
    } else {
        lines.join("\n")
    }
}

fn parse_status(text: &str) -> Status {
    let mut status = Status::default();
    let mut entries = text.split('\0');
    while let Some(entry) = entries.next() {
        if let Some(header) = entry.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" if value != "(initial)" => {
                    status.commit = Some(value.chars().take(7).collect());
                }
                "branch.head" if value != "(detached)" => status.branch = Some(value.into()),
                "branch.upstream" => status.upstream = Some(value.into()),
                "branch.ab" => {
                    for part in value.split(' ') {
                        if let Some(ahead) = part.strip_prefix('+') {
                            status.ahead = ahead.parse().unwrap_or(0);
                        } else if let Some(behind) = part.strip_prefix('-') {
                            status.behind = behind.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let mut fields = entry.splitn(2, ' ');
        let kind = fields.next().unwrap_or("");
        let rest = fields.next().unwrap_or("");
        let file = match kind {
            // Ordinary: XY sub mH mI mW hH hI path.
            "1" => changed_file(rest, 8),
            // Renamed or copied: one more field, then the old path follows.
            "2" => changed_file(rest, 9).map(|file| FileStatus {
                original: entries.next().map(String::from),
                ..file
            }),
            "u" => rest.splitn(10, ' ').nth(9).map(|path| FileStatus {
                path: path.into(),
                staged: None,
                unstaged: Some(Change::Conflicted),
                original: None,
            }),
            "?" => Some(FileStatus {
                path: rest.into(),
                staged: None,
                unstaged: Some(Change::Untracked),
                original: None,
            }),
            _ => None,
        };
        status.files.extend(file);
    }
    status
}

/// A tracked file's entry: its XY code, then `path` after `fields` fields.
fn changed_file(rest: &str, fields: usize) -> Option<FileStatus> {
    let mut parts = rest.splitn(fields, ' ');
    let code = parts.next()?;
    let path = parts.nth(fields - 2)?;
    let mut code = code.chars();
    Some(FileStatus {
        path: path.into(),
        staged: code.next().and_then(Change::from_code),
        unstaged: code.next().and_then(Change::from_code),
        original: None,
    })
}

fn parse_branches(text: &str) -> Vec<Branch> {
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for line in text.lines() {
        let Some((head, refname)) = line.split_once('\t') else {
            continue;
        };
        if let Some(name) = refname.strip_prefix("refs/heads/") {
            local.push(Branch {
                name: name.into(),
                remote: false,
                current: head == "*",
            });
        } else if let Some(name) = refname.strip_prefix("refs/remotes/") {
            // `origin/HEAD` only points at another branch.
            if !name.ends_with("/HEAD") && name.contains('/') {
                remote.push(Branch {
                    name: name.into(),
                    remote: true,
                    current: false,
                });
            }
        }
    }
    remote.retain(|branch: &Branch| {
        let short = branch.name.split_once('/').map_or("", |(_, name)| name);
        !local.iter().any(|local: &Branch| local.name == short)
    });
    local.extend(remote);
    local
}

/// How a run of lines in the buffer differs from the committed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    /// The buffer's lines, 0-based; empty for a deletion, which sits above
    /// `lines.start`.
    pub lines: Range<usize>,
    /// The committed lines these replace, each with its newline.
    pub old_text: String,
}

impl Hunk {
    pub fn is_deletion(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn is_addition(&self) -> bool {
        self.old_text.is_empty()
    }
}

/// The runs of lines `current` changes in `base`. A diff that takes too long
/// settles for a coarser answer rather than holding up typing.
pub fn hunks(base: &str, current: &str) -> Vec<Hunk> {
    // A checkout can write CRLF where the editor holds LF.
    let base = if current.contains('\r') {
        std::borrow::Cow::Borrowed(base)
    } else {
        std::borrow::Cow::Owned(base.replace("\r\n", "\n"))
    };
    let diff = similar::TextDiff::configure()
        .algorithm(similar::Algorithm::Myers)
        .timeout(Duration::from_millis(50))
        .diff_lines(base.as_ref(), current);
    let old_lines: Vec<&str> = base.split_inclusive('\n').collect();
    diff.grouped_ops(0)
        .into_iter()
        .filter_map(|group| {
            let first = group.first()?;
            let last = group.last()?;
            let old = first.old_range().start..last.old_range().end;
            let new = first.new_range().start..last.new_range().end;
            if old.is_empty() && new.is_empty() {
                return None;
            }
            Some(Hunk {
                lines: new,
                old_text: old_lines[old].concat(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunks_mark_added_changed_and_deleted_lines() {
        let base = "a\nb\nc\nd\ne\n";
        assert_eq!(hunks(base, base), vec![]);
        assert_eq!(
            hunks(base, "a\nb\nnew\nc\nd\ne\n"),
            vec![Hunk {
                lines: 2..3,
                old_text: String::new()
            }]
        );
        assert_eq!(
            hunks(base, "a\nB\nc\nd\ne\n"),
            vec![Hunk {
                lines: 1..2,
                old_text: "b\n".into()
            }]
        );
        let deleted = hunks(base, "a\nd\ne\n");
        assert_eq!(
            deleted,
            vec![Hunk {
                lines: 1..1,
                old_text: "b\nc\n".into()
            }]
        );
        assert!(deleted[0].is_deletion());
        // At the end, and two runs at once.
        assert_eq!(hunks(base, "a\nb\nc\n").len(), 1);
        assert_eq!(hunks(base, "A\nb\nc\nd\nE\n").len(), 2);
    }

    #[test]
    fn crlf_checkouts_compare_with_lf_buffers() {
        assert_eq!(hunks("a\r\nb\r\n", "a\nb\n"), vec![]);
        assert_eq!(hunks("a\r\nb\r\n", "a\r\nb\r\n"), vec![]);
    }

    #[test]
    fn reads_porcelain_status() {
        let text = [
            "# branch.oid 1234567890abcdef",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "1 M. N... 100644 100644 100644 aaaa bbbb src/staged file.rs",
            "1 .M N... 100644 100644 100644 aaaa bbbb README.md",
            "1 MM N... 100644 100644 100644 aaaa bbbb both.rs",
            "2 R. N... 100644 100644 100644 aaaa bbbb R100 new.rs",
            "old.rs",
            "u UU N... 100644 100644 100644 100644 aaaa bbbb cccc conflict.rs",
            "? notes/todo.txt",
            "",
        ]
        .join("\0");
        let status = parse_status(&text);
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert_eq!(status.commit.as_deref(), Some("1234567"));
        assert_eq!(status.upstream.as_deref(), Some("origin/main"));
        assert_eq!((status.ahead, status.behind), (2, 1));
        let files: Vec<_> = status
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.staged, f.unstaged))
            .collect();
        assert_eq!(
            files,
            vec![
                ("src/staged file.rs", Some(Change::Modified), None),
                ("README.md", None, Some(Change::Modified)),
                ("both.rs", Some(Change::Modified), Some(Change::Modified)),
                ("new.rs", Some(Change::Renamed), None),
                ("conflict.rs", None, Some(Change::Conflicted)),
                ("notes/todo.txt", None, Some(Change::Untracked)),
            ]
        );
        assert_eq!(status.files[3].original.as_deref(), Some("old.rs"));
        assert!(status.files[5].is_untracked());
    }

    #[test]
    fn a_detached_head_and_a_fresh_repository() {
        let status = parse_status("# branch.oid (initial)\0# branch.head (detached)\0");
        assert_eq!(status.branch, None);
        assert_eq!(status.commit, None);
    }

    #[test]
    fn remote_branches_with_a_local_one_are_left_out() {
        let text = "*\trefs/heads/main\n \trefs/heads/feature\n \trefs/remotes/origin/HEAD\n \trefs/remotes/origin/main\n \trefs/remotes/origin/other\n";
        let names: Vec<_> = parse_branches(text)
            .into_iter()
            .map(|b| (b.name, b.remote, b.current))
            .collect();
        assert_eq!(
            names,
            vec![
                ("main".into(), false, true),
                ("feature".into(), false, false),
                ("origin/other".into(), true, false),
            ]
        );
    }

    #[test]
    fn explains_failures_without_hints() {
        assert_eq!(
            explain(
                "error: failed to push\nhint: try pulling\nfatal: no way\n",
                ""
            ),
            "failed to push\nno way"
        );
        assert_eq!(explain("", "nothing to commit\n"), "nothing to commit");
    }

    /// A real repository: commit, change, stage, unstage, branch.
    #[test]
    fn works_with_a_real_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let run = |args: &[&str]| {
            let output = command(&root).args(args).output().unwrap();
            assert!(output.status.success(), "{args:?}: {output:?}");
        };
        run(&["init", "--quiet", "--initial-branch=main"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test"]);
        // Windows runners turn line endings into CRLF on checkout.
        run(&["config", "core.autocrlf", "false"]);
        let file = root.join("a.txt");
        std::fs::write(&file, "one\ntwo\n").unwrap();
        let repo = Repo::discover(&file).unwrap();
        assert_eq!(repo.root(), root);
        assert_eq!(repo.committed_text(&file), None);
        assert_eq!(
            repo.status().unwrap().files[0].unstaged,
            Some(Change::Untracked)
        );

        repo.stage(&["a.txt".into()]).unwrap();
        repo.unstage(&["a.txt".into()]).unwrap();
        assert_eq!(
            repo.status().unwrap().files[0].unstaged,
            Some(Change::Untracked)
        );
        let a = vec!["a.txt".to_string()];
        assert!(repo.commit("  ", &a).is_err());
        assert!(repo.commit("First", &[]).is_err());
        repo.commit("First", &a).unwrap();
        assert_eq!(repo.committed_text(&file).as_deref(), Some("one\ntwo\n"));
        let status = repo.status().unwrap();
        assert_eq!(status.branch.as_deref(), Some("main"));
        assert!(status.files.is_empty());

        std::fs::write(&file, "one\n2\n").unwrap();
        repo.stage(&["a.txt".into()]).unwrap();
        assert_eq!(
            repo.status().unwrap().files[0].staged,
            Some(Change::Modified)
        );
        repo.unstage(&["a.txt".into()]).unwrap();
        assert_eq!(repo.status().unwrap().files[0].staged, None);

        // A folder of untracked files is one entry.
        std::fs::create_dir(root.join("build")).unwrap();
        for name in ["x", "y", "z"] {
            std::fs::write(root.join("build").join(name), name).unwrap();
        }
        let paths = |repo: &Repo| -> Vec<String> {
            let status = repo.status().unwrap();
            status.files.into_iter().map(|f| f.path).collect()
        };
        assert_eq!(paths(&repo), vec!["a.txt", "build/"]);
        repo.stage(&["build/".into()]).unwrap();
        assert_eq!(repo.status().unwrap().files.len(), 4);
        repo.unstage(&["build/".into()]).unwrap();
        assert_eq!(paths(&repo), vec!["a.txt", "build/"]);
        // One with thousands is build output, and stays out.
        for n in 0..=MAX_NEW_FOLDER_FILES {
            std::fs::write(root.join("build").join(format!("f{n}")), "").unwrap();
        }
        let error = repo.stage(&["build/".into()]).unwrap_err().to_string();
        assert!(error.starts_with("build/ holds 1004 new files"), "{error}");
        assert!(repo.commit("Build", &["build/".into()]).is_err());
        assert_eq!(paths(&repo), vec!["a.txt", "build/"]);
        std::fs::remove_dir_all(root.join("build")).unwrap();

        // Only the files named are committed; another staged one stays staged.
        std::fs::write(root.join("b.txt"), "b\n").unwrap();
        std::fs::write(root.join("c.txt"), "c\n").unwrap();
        repo.stage(&["c.txt".into()]).unwrap();
        repo.commit("Add b", &["b.txt".into()]).unwrap();
        let status = repo.status().unwrap();
        let left: Vec<_> = status
            .files
            .iter()
            .map(|f| (f.path.as_str(), f.staged))
            .collect();
        assert_eq!(left, vec![("a.txt", None), ("c.txt", Some(Change::Added))]);
        repo.unstage(&["c.txt".into()]).unwrap();
        std::fs::remove_file(root.join("c.txt")).unwrap();

        // Another git holding the index for a moment only delays staging.
        let lock = root.join(".git/index.lock");
        std::fs::write(&lock, "").unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            std::fs::remove_file(lock).unwrap();
        });
        repo.stage(&["a.txt".into()]).unwrap();
        release.join().unwrap();
        repo.unstage(&["a.txt".into()]).unwrap();

        repo.create_branch("feature").unwrap();
        assert_eq!(repo.status().unwrap().branch.as_deref(), Some("feature"));
        let main = Branch {
            name: "main".into(),
            remote: false,
            current: false,
        };
        repo.switch(&main).unwrap();
        let branches = repo.branches().unwrap();
        assert!(branches.iter().any(|b| b.name == "main" && b.current));
        assert!(repo.pull().unwrap_err().to_string().contains("no upstream"));
        assert!(repo.push().unwrap_err().to_string().contains("no remote"));
    }
}
