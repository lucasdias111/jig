//! Run the built-in jigs on real code and grade what comes back.
//!
//!     cargo run -p jig-ai --example eval                      # every case, the quick model
//!     cargo run -p jig-ai --example eval -- --only explain    # cases whose id contains it
//!     cargo run -p jig-ai --example eval -- --model opencode-go/qwen3.8-flash
//!     cargo run -p jig-ai --example eval -- --judge opencode-go/glm-5.1
//!     cargo run -p jig-ai --example eval -- --no-rules        # without AGENTS.md
//!     cargo run -p jig-ai --example eval -- --compile         # build and test each Simplify
//!     cargo run -p jig-ai --example eval -- --dry-run         # print each selection, call nothing
//!     cargo run -p jig-ai --example eval -- --rejudge target/jig-evals/<run>
//!                                     # grade a run's replies again, no model calls
//!     cargo run -p jig-ai --example eval -- --no-judge        # only the checks (and --compile)
//!     cargo run -p jig-ai --example eval -- --check-cases     # planted bugs fail, the rest build
//!     cargo run -p jig-ai --example eval -- --list-models
//!
//! The cases are in `crates/jig-ai/evals/cases.toml`. Their code is read
//! from the commit the file names, so line numbers stay put however the
//! code moves on; the jigs' instructions come from `assets/default-jigs.toml`
//! and the prompt from this checkout, so a change to either is what's being
//! measured. Each run is written to `target/jig-evals/<time>-<model>/`:
//! `results.jsonl`, one line per case, and `report.md`.
//!
//! Every reply gets checks that need no model (Explain leaves the code as
//! it was, Add docs changes nothing but comments, Simplify changes code,
//! the message is at most 20 words), then a judge model grades it against
//! the case's `must` and `must_not` lists. With `--compile`, each edit is
//! put into a worktree of that commit and its crate built and its tests
//! run: Simplify and Extract function must keep them passing, Fix must
//! make a planted bug's tests pass, and Add tests must write tests that
//! run and pass. That says more than any judge.
//!
//! These call real models and cost money: don't run them in a loop.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use jig_ai::{Config, PromptRequest, Provider};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct Cases {
    /// The commit every case's code is read from.
    rev: String,
    /// The judge's `provider/model`, unless `--judge` says otherwise.
    judge: Option<String>,
    #[serde(rename = "case")]
    cases: Vec<Case>,
}

#[derive(Deserialize, Clone)]
struct Case {
    id: String,
    /// A jig's name in `assets/default-jigs.toml`.
    jig: String,
    file: String,
    /// `12-40`, counted from 1, both included. The selection runs from the
    /// start of the first line to the end of the last, without its newline,
    /// as selecting whole lines in the editor makes it. `end` is the cursor
    /// at the end of the file, for jigs that insert.
    lines: String,
    /// Cut the file at the line starting with this, e.g. `#[cfg(test)]`,
    /// so Add tests writes a module that isn't there yet.
    cut_from: Option<String>,
    /// Bugs to plant before the jig runs, as `[found, replaced]` pairs,
    /// each found exactly once: what Fix is meant to find.
    #[serde(default)]
    plant: Vec<[String; 2]>,
    /// What the language server would report, as Fix is sent it.
    #[serde(default)]
    diagnostics: Vec<CaseDiagnostic>,
    /// Take the selection's own comments out first, as if the code had
    /// none: Add docs then has something to do, and the author's comments
    /// are what the judge compares against.
    #[serde(default)]
    strip_comments: bool,
    /// Simplify may rightly leave the code as it is.
    #[serde(default)]
    may_stay: bool,
    /// The note typed after a colon, `docs: terse`.
    note: Option<String>,
    /// What a correct reply says or does.
    #[serde(default)]
    must: Vec<String>,
    /// Claims or changes that would make it wrong or misleading.
    #[serde(default)]
    must_not: Vec<String>,
    /// For `--compile`: the crate to build and the test filter to run.
    #[serde(rename = "crate")]
    krate: Option<String>,
    test: Option<String>,
}

#[derive(Deserialize, Clone)]
struct CaseDiagnostic {
    line: usize,
    #[serde(default = "error")]
    severity: String,
    message: String,
}

fn error() -> String {
    "error".into()
}

#[derive(Deserialize)]
struct Jigs {
    jig: Vec<Jig>,
}

#[derive(Deserialize)]
struct Jig {
    name: String,
    prompt: String,
    #[serde(default)]
    answer: bool,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct Outcome {
    id: String,
    jig: String,
    file: String,
    lines: String,
    seconds: f64,
    error: Option<String>,
    message: String,
    original: String,
    replace: String,
    /// Checks that need no model, by name: `None` passed.
    checks: BTreeMap<String, Option<String>>,
    judge: Option<Verdict>,
    /// From `--compile`: `None` built and passed its tests. Left out of
    /// the JSON when not built, so `null` there means it passed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    compiled: Option<Option<String>>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct Verdict {
    /// `pass`, `weak` or `fail`.
    verdict: String,
    /// Whether it answered what the jig asks about the selection, rather
    /// than something nearby.
    #[serde(default)]
    on_target: bool,
    #[serde(default)]
    accurate: bool,
    #[serde(default)]
    missed: Vec<String>,
    #[serde(default)]
    wrong: Vec<String>,
    #[serde(default)]
    note: String,
}

impl Outcome {
    fn failed_checks(&self) -> Vec<String> {
        self.checks
            .iter()
            .filter_map(|(name, problem)| problem.as_ref().map(|p| format!("{name}: {p}")))
            .collect()
    }

    /// Failed checks that make a reply wrong, not just long: too many
    /// words only makes it weak.
    fn wrong_checks(&self) -> usize {
        self.checks
            .iter()
            .filter(|(name, problem)| problem.is_some() && !name.contains(" words"))
            .count()
    }

    /// `pass`, `weak` or `fail`, everything considered.
    fn grade(&self) -> &'static str {
        if self.judge.as_ref().is_some_and(|v| v.verdict == "ungraded") && self.error.is_none() {
            return "ungraded";
        }
        if self.error.is_some() || self.wrong_checks() > 0 {
            return "fail";
        }
        if matches!(&self.compiled, Some(Some(_))) {
            return "fail";
        }
        let long = !self.failed_checks().is_empty();
        match self.judge.as_ref().map(|v| v.verdict.as_str()) {
            Some("pass") if long => "weak",
            Some("pass") => "pass",
            Some("weak") => "weak",
            Some(_) => "fail",
            None => "ungraded",
        }
    }
}

struct Options {
    model: Option<String>,
    judge: Option<String>,
    only: Vec<String>,
    jobs: usize,
    rules: bool,
    compile: bool,
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |name: &str| {
        args.iter()
            .position(|arg| arg == name)
            .and_then(|ix| args.get(ix + 1))
            .cloned()
    };
    let config = Config::load(Config::user_path().as_deref())?;
    if args.iter().any(|arg| arg == "--list-models") {
        for provider in &config.providers {
            let key = provider.system_api_key().map(|(key, _)| key);
            match provider.list_models(key.as_deref()) {
                Ok(models) => models
                    .iter()
                    .for_each(|model| println!("{}/{model}", provider.name)),
                Err(error) => println!("{}: {error}", provider.name),
            }
        }
        return Ok(());
    }
    let options = Options {
        model: value("--model"),
        judge: value("--judge"),
        only: args
            .iter()
            .enumerate()
            .filter(|(ix, _)| *ix > 0 && args[ix - 1] == "--only")
            .map(|(_, arg)| arg.clone())
            .collect(),
        jobs: value("--jobs").and_then(|n| n.parse().ok()).unwrap_or(4),
        rules: !args.iter().any(|arg| arg == "--no-rules"),
        compile: args.iter().any(|arg| arg == "--compile"),
    };

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = root.canonicalize()?;
    let cases: Cases = toml::from_str(&std::fs::read_to_string(
        root.join("crates/jig-ai/evals/cases.toml"),
    )?)?;
    let jigs: Jigs = toml::from_str(&std::fs::read_to_string(
        root.join("assets/default-jigs.toml"),
    )?)?;
    let selected: Vec<Case> = cases
        .cases
        .iter()
        .filter(|case| {
            options.only.is_empty() || options.only.iter().any(|only| case.id.contains(only))
        })
        .cloned()
        .collect();
    if selected.is_empty() {
        bail!("No case matches {:?}.", options.only);
    }

    if args.iter().any(|arg| arg == "--check-cases") {
        let source = Source {
            root: &root,
            rev: &cases.rev,
            jigs: &jigs,
            rules: None,
        };
        return check_cases(&source, &selected);
    }
    if args.iter().any(|arg| arg == "--dry-run") {
        for case in &selected {
            let request = request_for(case, &root, &cases.rev, &jigs, None)?;
            println!("── {} ({} {} {})", case.id, case.jig, case.file, case.lines);
            println!("{}", request.target_text());
        }
        return Ok(());
    }

    let earlier = value("--rejudge")
        .map(|dir| -> Result<Vec<Outcome>> {
            let text = std::fs::read_to_string(Path::new(&dir).join("results.jsonl"))?;
            text.lines()
                .map(|line| Ok(serde_json::from_str(line)?))
                .collect()
        })
        .transpose()?;
    let model_config = match &options.model {
        Some(id) => config.resolve(id)?,
        None => config.quick()?,
    };
    let judge_config = match &options.judge {
        Some(id) => config.resolve(id)?,
        // The agent's model, usually a bigger one than Jig mode's.
        None => match &cases.judge {
            Some(id) => config.resolve(id)?,
            None => match config.agent()? {
                jig_ai::AgentModel::Jig { provider, .. } => provider,
                jig_ai::AgentModel::OpenCode(id) => config.resolve(&id)?,
            },
        },
    };
    // A rejudge doesn't call the model, and --no-judge doesn't call the
    // judge, so neither needs a key then.
    let model = match &earlier {
        Some(_) => None,
        None => Some(model_config.build_with(api_key(&model_config))?),
    };
    let judge = match args.iter().any(|arg| arg == "--no-judge") {
        true => None,
        false => Some(judge_config.build_with(api_key(&judge_config))?),
    };
    let model_name = format!("{}/{}", model_config.name, model_config.model);
    let judge_name = format!("{}/{}", judge_config.name, judge_config.model);
    println!(
        "{} cases at {} · model {model_name} · judge {judge_name}{}",
        selected.len(),
        cases.rev,
        if options.rules {
            ""
        } else {
            " · no AGENTS.md"
        }
    );

    let rules = options
        .rules
        .then(|| git_show(&root, &cases.rev, "AGENTS.md").map(|text| agents_rules(&text)))
        .transpose()?;

    let earlier = Arc::new(earlier);

    // Run the cases a few at a time; each is a model call and a judge call.
    let queue = Arc::new(Mutex::new(selected.clone().into_iter().enumerate()));
    let outcomes = Arc::new(Mutex::new(vec![Outcome::default(); selected.len()]));
    std::thread::scope(|scope| {
        for _ in 0..options.jobs.max(1) {
            let (queue, outcomes, earlier) = (queue.clone(), outcomes.clone(), earlier.clone());
            let (model, judge) = (model.clone(), judge.clone());
            let source = Source {
                root: &root,
                rev: &cases.rev,
                jigs: &jigs,
                rules: rules.as_deref(),
            };
            scope.spawn(move || {
                loop {
                    let Some((ix, case)) = queue.lock().unwrap().next() else {
                        return;
                    };
                    let before = earlier
                        .as_ref()
                        .as_ref()
                        .and_then(|all| all.iter().find(|o| o.id == case.id));
                    let outcome =
                        run_case(&case, &source, model.as_deref(), judge.as_deref(), before);
                    println!(
                        "{:<8} {:<28} {:>5.1}s  {}",
                        outcome.grade(),
                        outcome.id,
                        outcome.seconds,
                        one_line(&outcome.message)
                    );
                    outcomes.lock().unwrap()[ix] = outcome;
                }
            });
        }
    });
    let mut outcomes = Arc::try_unwrap(outcomes)
        .ok()
        .context("threads finished")?
        .into_inner()?;

    if options.compile {
        let source = Source {
            root: &root,
            rev: &cases.rev,
            jigs: &jigs,
            rules: None,
        };
        compile_all(&source, &selected, &mut outcomes)?;
    }

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let out = root
        .join("target/jig-evals")
        .join(format!("{stamp}-{}", model_name.replace(['/', ':'], "_")));
    std::fs::create_dir_all(&out)?;
    let mut jsonl = String::new();
    for outcome in &outcomes {
        jsonl.push_str(&serde_json::to_string(outcome)?);
        jsonl.push('\n');
    }
    std::fs::write(out.join("results.jsonl"), jsonl)?;
    let report = report(
        &outcomes,
        &model_name,
        &judge_name,
        &cases.rev,
        options.rules,
    );
    std::fs::write(out.join("report.md"), &report)?;
    println!("\n{}", summary(&outcomes));
    println!("Written to {}", out.display());
    Ok(())
}

/// Where a case's request comes from.
struct Source<'a> {
    root: &'a Path,
    rev: &'a str,
    jigs: &'a Jigs,
    /// AGENTS.md as sent, unless `--no-rules`.
    rules: Option<&'a str>,
}

fn run_case(
    case: &Case,
    source: &Source,
    model: Option<&dyn Provider>,
    judge: Option<&dyn Provider>,
    earlier: Option<&Outcome>,
) -> Outcome {
    let mut outcome = Outcome {
        id: case.id.clone(),
        jig: case.jig.clone(),
        file: case.file.clone(),
        lines: case.lines.clone(),
        ..Outcome::default()
    };
    let request = match request_for(case, source.root, source.rev, source.jigs, source.rules) {
        Ok(request) => request,
        Err(error) => {
            outcome.error = Some(format!("case: {error:#}"));
            return outcome;
        }
    };
    outcome.original = request.target_text().to_string();
    if let Some(earlier) = earlier.filter(|earlier| earlier.error.is_none()) {
        // Grade the reply already given again, with today's checks and judge.
        outcome.seconds = earlier.seconds;
        outcome.message = earlier.message.clone();
        outcome.replace = earlier.replace.clone();
        outcome.checks = checks(case, &outcome, comment_marker(&case.file));
        outcome.judge = judged(judge, case, &request, &outcome);
        return outcome;
    }
    let Some(model) = model else {
        outcome.error = Some("not in the run being graded again".into());
        return outcome;
    };
    let started = Instant::now();
    let reply = jig_ai::run(model, &request);
    outcome.seconds = started.elapsed().as_secs_f64();
    let reply = match reply {
        Ok(reply) => reply,
        Err(error) => {
            outcome.error = Some(format!("{error:#}"));
            return outcome;
        }
    };
    outcome.message = reply.message;
    outcome.replace = reply.replace;
    outcome.checks = checks(case, &outcome, comment_marker(&case.file));
    outcome.judge = judged(judge, case, &request, &outcome);
    outcome
}

fn judged(
    judge: Option<&dyn Provider>,
    case: &Case,
    request: &PromptRequest,
    outcome: &Outcome,
) -> Option<Verdict> {
    let judge = judge?;
    // A judge's reply now and then comes back cut short; once more is
    // usually enough.
    let verdict =
        grade(judge, case, request, outcome).or_else(|_| grade(judge, case, request, outcome));
    Some(verdict.unwrap_or_else(|error| Verdict {
        verdict: "ungraded".into(),
        note: format!("judge: {error:#}"),
        ..Verdict::default()
    }))
}

/// The provider's key as Jig finds it, else, on macOS, through `security`:
/// the keychain asks again for every new build of this example, but once
/// allowed for `security` it stays allowed.
fn api_key(provider: &jig_ai::ProviderConfig) -> Option<String> {
    if let Some((key, _)) = provider.system_api_key() {
        return Some(key);
    }
    let output = Command::new("security")
        .args([
            "find-generic-password",
            "-s",
            "Jig",
            "-a",
            &provider.name,
            "-w",
        ])
        .output()
        .ok()?;
    let key = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (output.status.success() && !key.is_empty()).then_some(key)
}

/// The request the editor would send for `case`.
fn request_for(
    case: &Case,
    root: &Path,
    rev: &str,
    jigs: &Jigs,
    rules: Option<&str>,
) -> Result<PromptRequest> {
    let jig = jigs
        .jig
        .iter()
        .find(|jig| jig.name == case.jig)
        .with_context(|| format!("no jig named {}", case.jig))?;
    let mut text = git_show(root, rev, &case.file)?;
    if let Some(marker) = &case.cut_from {
        let at = text
            .match_indices('\n')
            .map(|(ix, _)| ix + 1)
            .find(|&ix| text[ix..].starts_with(marker.as_str()))
            .with_context(|| format!("no line starts with {marker}"))?;
        text.truncate(at);
        text.truncate(text.trim_end().len());
        text.push_str("\n\n");
    }
    for [found, replaced] in &case.plant {
        if text.matches(found.as_str()).count() != 1 {
            bail!("`{found}` isn't in {} exactly once", case.file);
        }
        text = text.replacen(found.as_str(), replaced, 1);
    }
    let mut target = if case.lines == "end" {
        text.len()..text.len()
    } else {
        line_range(&text, &case.lines)?
    };
    if case.strip_comments {
        let marker = comment_marker(&case.file);
        let stripped = strip_comment_lines(&text[target.clone()], marker);
        text = format!("{}{stripped}{}", &text[..target.start], &text[target.end..]);
        target = target.start..target.start + stripped.len();
    }
    Ok(PromptRequest {
        instruction: jig.prompt.clone(),
        comment: case.note.clone(),
        project_rules: rules.map(str::to_string),
        language: language(&case.file).into(),
        file_name: Some(case.file.clone()),
        text,
        target,
        diagnostics: case
            .diagnostics
            .iter()
            .map(|d| jig_ai::prompt::Diagnostic {
                line: d.line,
                severity: d.severity.clone(),
                message: d.message.clone(),
            })
            .collect(),
        task: None,
        answer: jig.answer,
    })
}

/// AGENTS.md as the editor sends it: trimmed, cut at 16 KB.
fn agents_rules(text: &str) -> String {
    const MAX: usize = 16 * 1024;
    let text = text.trim();
    if text.len() <= MAX {
        return text.to_string();
    }
    let mut end = MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n[AGENTS.md was cut here; it is longer than Jig sends.]",
        &text[..end]
    )
}

fn git_show(root: &Path, rev: &str, file: &str) -> Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["show", &format!("{rev}:{file}")])
        .output()?;
    if !output.status.success() {
        bail!(
            "git show {rev}:{file}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8(output.stdout)?)
}

fn line_range(text: &str, lines: &str) -> Result<std::ops::Range<usize>> {
    let (first, last) = lines.split_once('-').unwrap_or((lines, lines));
    let (first, last): (usize, usize) = (first.trim().parse()?, last.trim().parse()?);
    let starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(ix, _)| ix + 1))
        .collect();
    if first == 0 || last < first || last >= starts.len() {
        bail!("lines {lines} aren't in the file");
    }
    Ok(starts[first - 1]..starts[last] - 1)
}

fn language(file: &str) -> &'static str {
    match Path::new(file).extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("sh") => "shell",
        Some("toml") => "toml",
        Some("py") => "python",
        Some("ts") => "typescript",
        _ => "text",
    }
}

fn comment_marker(file: &str) -> &'static str {
    match language(file) {
        "shell" | "toml" | "python" => "#",
        _ => "//",
    }
}

fn is_comment_line(line: &str, marker: &str) -> bool {
    let line = line.trim_start();
    // A shebang is code.
    line.starts_with(marker) && !line.starts_with("#!")
}

fn strip_comment_lines(text: &str, marker: &str) -> String {
    text.split_inclusive('\n')
        .filter(|line| !is_comment_line(line, marker))
        .collect::<String>()
        .trim_end_matches('\n')
        .to_string()
        + if text.ends_with('\n') { "\n" } else { "" }
}

/// `text` with every comment and blank line gone, and each line trimmed:
/// what has to stay the same when only docs were meant to change.
fn code_only(text: &str, marker: &str) -> Vec<String> {
    text.lines()
        .filter(|line| !is_comment_line(line, marker))
        .map(|line| without_trailing_comment(line, marker).trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

/// `line` without a `// comment` at its end, outside strings. Only for
/// `//`: a `#` too often means something else.
fn without_trailing_comment<'a>(line: &'a str, marker: &str) -> &'a str {
    if marker != "//" {
        return line;
    }
    let bytes = line.as_bytes();
    let mut in_string = false;
    let mut ix = 0;
    while ix < bytes.len() {
        match bytes[ix] {
            b'\\' if in_string => ix += 1,
            b'"' => in_string = !in_string,
            b'/' if !in_string && bytes.get(ix + 1) == Some(&b'/') => return &line[..ix],
            _ => {}
        }
        ix += 1;
    }
    line
}

fn comment_lines(text: &str, marker: &str) -> usize {
    text.lines()
        .filter(|line| is_comment_line(line, marker))
        .count()
}

/// Checks that need no model. `None` is a pass.
fn checks(case: &Case, outcome: &Outcome, marker: &str) -> BTreeMap<String, Option<String>> {
    let mut checks = BTreeMap::new();
    let words = outcome.message.split_whitespace().count();
    let (limit, label) = if case.jig == "Explain" {
        (jig_ai::prompt::ANSWER_WORDS, "answer ≤ 80 words")
    } else {
        (jig_ai::prompt::MESSAGE_WORDS, "message ≤ 20 words")
    };
    checks.insert(
        label.into(),
        (words > limit).then(|| format!("{words} words")),
    );
    let (old, new) = (&outcome.original, &outcome.replace);
    match case.jig.as_str() {
        "Explain" => {
            checks.insert(
                "code unchanged".into(),
                (old != new).then(|| "the code was changed".into()),
            );
            checks.insert(
                "says something".into(),
                outcome
                    .message
                    .trim()
                    .is_empty()
                    .then(|| "no message".into()),
            );
        }
        "Add docs" => {
            let (before, after) = (code_only(old, marker), code_only(new, marker));
            checks.insert(
                "only comments changed".into(),
                (before != after).then(|| first_difference(&before, &after)),
            );
            checks.insert(
                "docs added".into(),
                (comment_lines(new, marker) <= comment_lines(old, marker)
                    && new.matches(marker).count() <= old.matches(marker).count())
                .then(|| "no new comments".into()),
            );
        }
        "Simplify" if !case.may_stay => {
            checks.insert(
                "code changed".into(),
                (code_only(old, marker) == code_only(new, marker))
                    .then(|| "only whitespace or comments changed, or nothing".into()),
            );
        }
        _ => {}
    }
    match case.jig.as_str() {
        "Extract function" => {
            checks.insert(
                "a function is added".into(),
                (new.matches("fn ").count() <= old.matches("fn ").count())
                    .then(|| "no new fn".into()),
            );
        }
        "Fix" => {
            checks.insert(
                "code changed".into(),
                (code_only(old, marker) == code_only(new, marker))
                    .then(|| "the code wasn't changed".into()),
            );
        }
        "Add tests" => {
            checks.insert(
                "tests added".into(),
                (!new.contains("#[test]")).then(|| "no #[test]".into()),
            );
        }
        _ => {}
    }
    // A rough count, fooled by brackets in strings and character literals:
    // only for cases that aren't built, where nothing better says.
    let edits = ["Simplify", "Extract function", "Fix", "Add tests"];
    if case.krate.is_none() && edits.contains(&case.jig.as_str()) {
        checks.insert(
            "brackets balance".into(),
            (bracket_balance(old) != bracket_balance(new))
                .then(|| "brackets don't balance as they did".into()),
        );
    }
    checks
}

fn first_difference(before: &[String], after: &[String]) -> String {
    let at = before
        .iter()
        .zip(after)
        .position(|(a, b)| a != b)
        .unwrap_or(before.len().min(after.len()));
    format!(
        "code line {}: `{}` became `{}`",
        at + 1,
        before.get(at).map_or("(none)", String::as_str),
        after.get(at).map_or("(none)", String::as_str)
    )
}

fn bracket_balance(text: &str) -> [i64; 3] {
    let mut balance = [0; 3];
    for c in text.chars() {
        match c {
            '(' => balance[0] += 1,
            ')' => balance[0] -= 1,
            '[' => balance[1] += 1,
            ']' => balance[1] -= 1,
            '{' => balance[2] += 1,
            '}' => balance[2] -= 1,
            _ => {}
        }
    }
    balance
}

const JUDGE_SYSTEM: &str = r#"You grade the output of Jig, a code editor's AI tool. The user selected some code and ran a command ("jig") on it. For an edit, the tool replies with the new text for the selection and a message of at most 20 words saying what it changed. For an explanation, it replies with an answer of at most 80 words and the code stays as it was.

Read the code yourself and decide whether the reply is correct. Be strict and concrete: a reply that is vague, explains or changes something other than the selection, or says something false about the code is not a pass. The case's checklist says what a good reply must do and what would make it wrong; use it, but also report any other error you find.

For Fix, a bug was planted in the selection (and diagnostics may name it): the reply must fix it without other changes. For Extract function, the new function and its call replace the selection. For Add tests, the reply is inserted at the cursor and its tests must be right about the code.

Grades:
- "pass": correct, about the selected code, and does what the jig asks.
- "weak": nothing false, but it misses the point, is too generic to help, or does less than asked.
- "fail": says or does something wrong or misleading, has a message that describes a change the code doesn't make, answers a different question, changes behaviour (for Simplify), changes code (for Add docs or Explain), or writes docs that misdescribe the code.

Your reply may go through a tool with "replace" and "message" fields: then put the verdict object, as JSON text, in "replace" and leave "message" empty. Otherwise reply with the object itself. The verdict object:
{"verdict": "pass" | "weak" | "fail", "on_target": true/false, "accurate": true/false, "missed": ["must items it didn't meet"], "wrong": ["false or misleading claims, or behaviour changes"], "note": "one sentence"}"#;

fn grade(
    judge: &dyn Provider,
    case: &Case,
    request: &PromptRequest,
    outcome: &Outcome,
) -> Result<Verdict> {
    let mut user = String::new();
    writeln!(user, "Jig: {}", case.jig)?;
    writeln!(user, "Instruction sent: {}", request.instruction)?;
    if let Some(note) = &case.note {
        writeln!(user, "Note: {note}")?;
    }
    writeln!(user, "File: {} (lines {})", case.file, case.lines)?;
    writeln!(
        user,
        "\n<context_before>\n{}\n</context_before>",
        context(request, true)
    )?;
    writeln!(user, "\n<selection>\n{}\n</selection>", outcome.original)?;
    writeln!(
        user,
        "\n<context_after>\n{}\n</context_after>",
        context(request, false)
    )?;
    writeln!(user, "\n<checklist>")?;
    for item in &case.must {
        writeln!(user, "must: {item}")?;
    }
    for item in &case.must_not {
        writeln!(user, "must not: {item}")?;
    }
    writeln!(user, "</checklist>")?;
    writeln!(
        user,
        "\n<reply_message>\n{}\n</reply_message>",
        outcome.message
    )?;
    if outcome.replace == outcome.original {
        writeln!(user, "\n<reply_code>(unchanged)</reply_code>")?;
    } else {
        writeln!(user, "\n<reply_code>\n{}\n</reply_code>", outcome.replace)?;
    }
    writeln!(user, "\nReply with the JSON object only.")?;
    let raw = judge.complete(JUDGE_SYSTEM, &user)?;
    // Through Jig's forced reply tool, the verdict is the text of `replace`.
    let raw = match first_json::<serde_json::Value>(&raw) {
        Ok(serde_json::Value::Object(object)) if !object.contains_key("verdict") => object
            .get("replace")
            .and_then(serde_json::Value::as_str)
            .context("the judge returned neither a verdict nor a replace")?
            .to_string(),
        _ => raw,
    };
    first_json(&raw)
}

fn first_json<T: serde::de::DeserializeOwned>(raw: &str) -> Result<T> {
    let start = raw.find('{').context("the judge didn't return JSON")?;
    let mut stream = serde_json::Deserializer::from_str(&raw[start..]).into_iter::<T>();
    Ok(stream.next().context("the judge didn't return JSON")??)
}

/// Up to 60 lines before or after the selection, for the judge.
fn context(request: &PromptRequest, before: bool) -> String {
    // Code inserted at the cursor is about the whole file.
    let lines = if request.target.is_empty() { 400 } else { 60 };
    if before {
        let shown: Vec<&str> = request.text[..request.target.start].lines().collect();
        shown[shown.len().saturating_sub(lines)..].join("\n")
    } else {
        request.text[request.target.end..]
            .lines()
            .take(lines)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The worktree of `rev` replies are built in, made the first time.
fn worktree(root: &Path, rev: &str) -> Result<PathBuf> {
    let tree = root.join("target/jig-evals/tree");
    if !tree.exists() {
        let status = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["worktree", "add", "--detach"])
            .arg(&tree)
            .arg(rev)
            .status()?;
        if !status.success() {
            bail!("couldn't make a worktree at {}", tree.display());
        }
    }
    Ok(tree)
}

/// Build `case`'s crate with `file` in place of its file and run its
/// tests: `None` when they built, ran and passed.
fn build_and_test(root: &Path, rev: &str, case: &Case, file: &str) -> Result<Option<String>> {
    let Some(krate) = &case.krate else {
        return Ok(None);
    };
    let tree = worktree(root, rev)?;
    run_git(&tree, &["checkout", "--quiet", "--detach", "--force", rev])?;
    run_git(&tree, &["clean", "-fdq"])?;
    std::fs::write(tree.join(&case.file), file)?;
    let mut command = Command::new("cargo");
    command
        .current_dir(&tree)
        .env("CARGO_TARGET_DIR", root.join("target/jig-evals/target"))
        .args(["test", "--quiet", "-p", krate]);
    if let Some(filter) = &case.test {
        command.arg(filter);
    }
    let output = command.output()?;
    run_git(&tree, &["checkout", "--quiet", "--detach", "--force", rev])?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    if output.status.success() {
        // A filter that matches nothing passes too; written tests must run.
        let passed: usize = stdout
            .lines()
            .filter_map(|line| line.strip_prefix("test result: ok. "))
            .filter_map(|rest| rest.split(' ').next()?.parse::<usize>().ok())
            .sum();
        return Ok((passed == 0 && case.jig == "Add tests").then(|| "no tests ran".into()));
    }
    let lines: Vec<&str> = stderr
        .lines()
        .chain(stdout.lines())
        .filter(|line| {
            line.starts_with("error") || line.contains("panicked") || line.contains("FAILED")
        })
        .take(6)
        .collect();
    Ok(Some(lines.join(" | ")))
}

/// Build each edit's file with the reply in it and run its tests.
fn compile_all(source: &Source, cases: &[Case], outcomes: &mut [Outcome]) -> Result<()> {
    for (case, outcome) in cases.iter().zip(outcomes.iter_mut()) {
        let edits = ["Simplify", "Extract function", "Fix", "Add tests"];
        if case.krate.is_none()
            || !edits.contains(&case.jig.as_str())
            || outcome.error.is_some()
            || outcome.replace == outcome.original
        {
            continue;
        }
        let request = request_for(case, source.root, source.rev, source.jigs, None)?;
        let file = format!(
            "{}{}{}",
            &request.text[..request.target.start],
            outcome.replace,
            &request.text[request.target.end..]
        );
        print!("compile  {:<28} ", case.id);
        let problem = build_and_test(source.root, source.rev, case, &file)?;
        println!("{}", problem.as_deref().unwrap_or("ok"));
        outcome.compiled = Some(problem);
    }
    Ok(())
}

/// Check the cases themselves, without a model: each planted bug fails
/// its tests (or the build), and everything else builds as it is.
fn check_cases(source: &Source, cases: &[Case]) -> Result<()> {
    let mut wrong = 0;
    for case in cases.iter().filter(|case| case.krate.is_some()) {
        let request = request_for(case, source.root, source.rev, source.jigs, None)?;
        let problem = build_and_test(source.root, source.rev, case, &request.text)?;
        let planted = !case.plant.is_empty();
        let fine = match (&problem, planted) {
            (Some(_), true) | (None, false) => true,
            // A file cut before its tests builds and runs none.
            (Some(p), false) => case.jig == "Add tests" && p == "no tests ran",
            (None, true) => false,
        };
        wrong += usize::from(!fine);
        println!(
            "{:<6} {:<28} {}",
            if fine { "ok" } else { "WRONG" },
            case.id,
            problem.as_deref().unwrap_or("builds and passes")
        );
    }
    if wrong > 0 {
        bail!("{wrong} cases don't do what they should");
    }
    Ok(())
}

fn run_git(dir: &Path, args: &[&str]) -> Result<()> {
    let status = Command::new("git").arg("-C").arg(dir).args(args).status()?;
    if !status.success() {
        bail!("git {} failed", args.join(" "));
    }
    Ok(())
}

fn one_line(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > 90 {
        text.chars().take(89).collect::<String>() + "…"
    } else {
        text
    }
}

fn summary(outcomes: &[Outcome]) -> String {
    let mut by_jig: BTreeMap<&str, BTreeMap<&str, usize>> = BTreeMap::new();
    for outcome in outcomes {
        *by_jig
            .entry(outcome.jig.as_str())
            .or_default()
            .entry(outcome.grade())
            .or_default() += 1;
    }
    let mut out = String::new();
    let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
    for (jig, grades) in &by_jig {
        let parts: Vec<String> = ["pass", "weak", "fail", "ungraded"]
            .iter()
            .filter_map(|g| grades.get(g).map(|n| format!("{n} {g}")))
            .collect();
        let _ = writeln!(out, "{jig:<10} {}", parts.join(", "));
        for (grade, n) in grades {
            *totals.entry(grade).or_default() += n;
        }
    }
    let parts: Vec<String> = ["pass", "weak", "fail", "ungraded"]
        .iter()
        .filter_map(|g| totals.get(g).map(|n| format!("{n} {g}")))
        .collect();
    let _ = write!(out, "{:<10} {}", "All", parts.join(", "));
    out
}

fn report(outcomes: &[Outcome], model: &str, judge: &str, rev: &str, rules: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Jig eval\n");
    let _ = writeln!(
        out,
        "Model `{model}`, judge `{judge}`, code at `{rev}`, AGENTS.md {}.\n",
        if rules { "sent" } else { "not sent" }
    );
    let _ = writeln!(out, "```\n{}\n```\n", summary(outcomes));
    for outcome in outcomes {
        let _ = writeln!(
            out,
            "## {} — {} ({} {}, {:.1}s)\n",
            outcome.id,
            outcome.grade(),
            outcome.file,
            outcome.lines,
            outcome.seconds
        );
        if let Some(error) = &outcome.error {
            let _ = writeln!(out, "Error: {error}\n");
            continue;
        }
        let _ = writeln!(out, "Message: {}\n", outcome.message);
        for problem in outcome.failed_checks() {
            let _ = writeln!(out, "- Check failed: {problem}");
        }
        if let Some(Some(problem)) = &outcome.compiled {
            let _ = writeln!(out, "- Build or tests failed: {problem}");
        }
        if let Some(verdict) = &outcome.judge {
            let _ = writeln!(
                out,
                "- Judge: {} (on target: {}, accurate: {}). {}",
                verdict.verdict, verdict.on_target, verdict.accurate, verdict.note
            );
            for missed in &verdict.missed {
                let _ = writeln!(out, "  - missed: {missed}");
            }
            for wrong in &verdict.wrong {
                let _ = writeln!(out, "  - wrong: {wrong}");
            }
        }
        if outcome.replace != outcome.original {
            let _ = writeln!(out, "\n```diff");
            for line in outcome.original.lines() {
                let _ = writeln!(out, "-{line}");
            }
            for line in outcome.replace.lines() {
                let _ = writeln!(out, "+{line}");
            }
            let _ = writeln!(out, "```");
        }
        let _ = writeln!(out);
    }
    out
}
