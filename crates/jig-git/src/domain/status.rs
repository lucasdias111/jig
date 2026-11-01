//! What git's output says: the status of the working tree, its branches,
//! and why a command failed.

/// Untracked folders with more files than this aren't staged.
pub const MAX_NEW_FOLDER_FILES: usize = 1000;

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

/// Git's message for a failure, without its hints, at most a few lines.
pub fn explain(stderr: &str, stdout: &str) -> String {
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

pub fn parse_status(text: &str) -> Status {
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
pub fn changed_file(rest: &str, fields: usize) -> Option<FileStatus> {
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

pub fn parse_branches(text: &str) -> Vec<Branch> {
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
