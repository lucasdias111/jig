//! Telling shell commands that only read from those that might change
//! something, so the agent can look around the project without asking.
//! Anything unsure counts as changing: the user is asked, as before.

mod shell;

use std::path::{Component, Path, PathBuf};

use shell::{simple_commands, words};

/// Programs that look and change nothing, given the checks in
/// [`reads_only`].
const READERS: &[&str] = &[
    "ls", "cat", "head", "tail", "grep", "egrep", "fgrep", "rg", "ag", "find", "fd", "wc", "sort",
    "uniq", "tree", "file", "stat", "pwd", "echo", "cut", "tr", "nl", "du", "basename", "dirname",
    "realpath", "readlink", "which", "awk", "sed", "jq", "column", "diff", "cd", "true", "printf",
    "xargs",
];

/// Git's subcommands that only read (`branch` only when it lists).
const GIT_READERS: &[&str] = &[
    "ls-files",
    "log",
    "show",
    "status",
    "diff",
    "grep",
    "blame",
    "branch",
    "rev-parse",
    "shortlog",
    "ls-tree",
    "cat-file",
];

/// Whether `command` only reads, run in `project`: every command in it is
/// one that looks and changes nothing, writing to no file, and nothing it
/// names lies outside the project.
pub fn command_only_reads(command: &str, project: &Path) -> bool {
    let Some(parts) = simple_commands(command) else {
        return false;
    };
    let mut cwd = project.to_path_buf();
    parts.iter().all(|part| {
        let Some(words) = words(part) else {
            return false;
        };
        let Some(program) = words.first() else {
            return true;
        };
        let program = program.rsplit('/').next().unwrap_or(program);
        if !reads_only(program, &words[1..]) {
            return false;
        }
        if !words[1..].iter().all(|word| inside(word, &cwd, project)) {
            return false;
        }
        if program == "cd" {
            match words.get(1) {
                Some(to) => cwd = resolve(&cwd, to),
                None => return false,
            }
        }
        true
    })
}

/// Whether `program` run with `args` only reads.
fn reads_only(program: &str, args: &[String]) -> bool {
    let has = |flags: &[&str]| {
        args.iter().any(|arg| {
            flags
                .iter()
                .any(|flag| arg == flag || arg.starts_with(&format!("{flag}=")))
        })
    };
    let operands = || args.iter().filter(|arg| !arg.starts_with('-'));
    match program {
        "git" => git_reads_only(args),
        "find" => !has(&[
            "-exec", "-execdir", "-delete", "-ok", "-okdir", "-fprint", "-fprint0", "-fprintf",
            "-fls",
        ]),
        "fd" => !has(&["-x", "-X", "--exec", "--exec-batch"]),
        "sed" => sed_reads_only(args),
        // It can write files and run commands from inside its program;
        // comparisons with `>` ask too, which is the price of not parsing it.
        "awk" => !args
            .iter()
            .any(|arg| arg.contains('>') || arg.contains('|') || arg.contains("system")),
        "xargs" => operands()
            .next()
            .is_some_and(|next| READERS.contains(&next.as_str()) && next != "xargs"),
        // Its preprocessor runs a command on each file.
        "rg" => !has(&["--pre"]),
        "sort" => !args.iter().any(|arg| {
            arg.starts_with("-o") || arg.starts_with("--output") || arg.starts_with("--compress")
        }),
        "tree" => !has(&["-o"]),
        "file" => !has(&["-C", "--compile"]),
        // A second operand is the file it writes.
        "uniq" => operands().count() <= 1,
        _ => READERS.contains(&program),
    }
}

fn git_reads_only(args: &[String]) -> bool {
    let Some(at) = args.iter().position(|arg| !arg.starts_with('-')) else {
        return false;
    };
    let (sub, rest) = (args[at].as_str(), &args[at + 1..]);
    if !GIT_READERS.contains(&sub) {
        return false;
    }
    let writes = rest.iter().any(|arg| {
        arg.starts_with("--output")
            || arg == "-O"
            || arg.starts_with("--open-files-in-pager")
            || arg == "--ext-diff"
    });
    // `git branch` lists unless it's given a name or told to change one.
    let changes_branch = sub == "branch"
        && rest.iter().any(|arg| {
            !arg.starts_with('-')
                || matches!(
                    arg.as_str(),
                    "-d" | "-D"
                        | "-m"
                        | "-M"
                        | "-c"
                        | "-C"
                        | "-f"
                        | "-u"
                        | "--delete"
                        | "--move"
                        | "--copy"
                        | "--force"
                        | "--unset-upstream"
                        | "--edit-description"
                )
                || arg.starts_with("--set-upstream")
        })
        && !rest.iter().any(|arg| arg == "--list" || arg == "-l");
    !writes && !changes_branch
}

/// Whether `sed` with `args` only prints: not in place, and every command
/// in its script one that prints or edits the stream (no `w`, `r`, `e`).
fn sed_reads_only(args: &[String]) -> bool {
    let mut scripts = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-e" | "--expression" => scripts.push(args.next().cloned().unwrap_or_default()),
            "-n" | "-E" | "-r" | "-s" | "-u" | "-z" | "--quiet" | "--silent" | "--posix"
            | "--regexp-extended" | "--separate" | "--unbuffered" | "--null-data" => {}
            _ if arg.starts_with("--expression=") => {
                scripts.push(arg["--expression=".len()..].to_string())
            }
            // In place, from a file of commands, or anything else unknown.
            _ if arg.starts_with('-') => {
                if !arg[1..].chars().all(|c| "nErsuz".contains(c)) {
                    return false;
                }
            }
            _ if scripts.is_empty() => scripts.push(arg.clone()),
            _ => {}
        }
    }
    !scripts.is_empty() && scripts.iter().all(|script| sed_script_reads(script))
}

/// Whether a sed script only prints and edits the stream.
fn sed_script_reads(script: &str) -> bool {
    let chars: Vec<char> = script.chars().collect();
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if c.is_whitespace() || matches!(c, ';' | '{' | '}') {
            at += 1;
            continue;
        }
        // The address: line numbers, steps, `$`, `!` and /regexes/.
        if c.is_ascii_digit() || matches!(c, '$' | ',' | '+' | '~' | '!') {
            at += 1;
            continue;
        }
        if c == '/' {
            match skip_field(&chars, at + 1, '/') {
                Some(end) => {
                    at = end;
                    // `I` and `M` after a regex address are its flags.
                    while at < chars.len() && matches!(chars[at], 'I' | 'M') {
                        at += 1;
                    }
                    continue;
                }
                None => return false,
            }
        }
        match c {
            'p' | 'P' | 'd' | 'D' | '=' | 'q' | 'Q' | 'n' | 'N' | 'l' | 'g' | 'G' | 'h' | 'H'
            | 'x' | 'z' => at += 1,
            's' | 'y' => {
                let Some(&delimiter) = chars.get(at + 1) else {
                    return false;
                };
                if delimiter == '\\' || delimiter == '\n' {
                    return false;
                }
                let Some(end) = skip_field(&chars, at + 2, delimiter)
                    .and_then(|end| skip_field(&chars, end, delimiter))
                else {
                    return false;
                };
                at = end;
                // Its flags; `w` writes and `e` runs.
                while at < chars.len() && "gpiImM0123456789".contains(chars[at]) {
                    at += 1;
                }
            }
            _ => return false,
        }
    }
    true
}

/// Where the field starting at `from` ends: just past its closing
/// `delimiter`, skipping escaped ones.
fn skip_field(chars: &[char], from: usize, delimiter: char) -> Option<usize> {
    let mut at = from;
    while at < chars.len() {
        match chars[at] {
            '\\' => at += 2,
            c if c == delimiter => return Some(at + 1),
            _ => at += 1,
        }
    }
    None
}

/// Whether `word`, if it names a path, names one in `project`. A path
/// outside it that doesn't exist (as a pattern like `/api/users` doesn't)
/// reads nothing there, so it's fine.
fn inside(word: &str, cwd: &Path, project: &Path) -> bool {
    let word = match word.split_once('=') {
        Some((flag, value)) if flag.starts_with('-') => value,
        _ => word,
    };
    if !word.contains('/') && word != ".." {
        return true;
    }
    if word == "/dev/null" {
        return true;
    }
    // Up to any glob, which the shell expands.
    let fixed = word
        .find(['*', '?', '[', '{'])
        .map_or(word, |glob| &word[..glob]);
    let path = resolve(cwd, fixed);
    if path.starts_with(project) || !path.exists() {
        return true;
    }
    // The same folder by another name, as `/private/var` is `/var`.
    let real = |path: &Path| path.canonicalize().ok();
    real(&path)
        .zip(real(project))
        .is_some_and(|(path, project)| path.starts_with(project))
}

/// `to` from `from`, `..` worked out without asking the disk.
fn resolve(from: &Path, to: &str) -> PathBuf {
    let mut path = from.to_path_buf();
    for component in Path::new(to).components() {
        match component {
            Component::RootDir | Component::Prefix(_) => {
                path = PathBuf::from(component.as_os_str())
            }
            Component::ParentDir => {
                path.pop();
            }
            Component::CurDir => {}
            Component::Normal(name) => path.push(name),
        }
    }
    path
}

#[cfg(test)]
mod tests;
