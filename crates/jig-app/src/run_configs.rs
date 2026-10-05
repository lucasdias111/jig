//! Run configurations: the commands a project is run, tested and built
//! with, as IntelliJ has them.
//!
//! They come from the project's `.jig/run.toml`, and from what the project
//! itself declares: Cargo binaries and examples, `package.json` scripts, a
//! Go module. A configuration in the file with the same name as a detected
//! one replaces it.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result};
use serde::Deserialize;

/// The project's own configurations, relative to its root.
pub const FILE: &str = ".jig/run.toml";

/// A starter for a new `.jig/run.toml`.
pub const TEMPLATE: &str = r#"# Run configurations for this project. Pick one with ⌃⌥R, run it with ⌃R,
# stop it with ⌘F2. Jig also lists the Cargo binaries, package.json scripts
# and Go module it finds; one here with the same name replaces it.
#
# command  runs through /bin/sh (cmd on Windows), from the project root unless `cwd` says
#          otherwise (relative to the root).
# env      is added to the environment.
#
# Debugging (⌃D) works on its own for commands a debugger takes, such as
# `cargo run`, `npm run dev`, `python3 app.py` or `go run .`. Otherwise:
# program   what to debug, relative to the root: an executable or a script;
# args      its arguments;
# build     a shell command that builds it first;
# debugger  which debugger to use, by its key in Settings.

[[run]]
name = "Run"
command = ""
# cwd = "."
# env = { RUST_LOG = "debug" }
# program = "target/debug/app"
# args = ["--verbose"]
# build = "make"
# debugger = "rust"
"#;

#[derive(Clone, Debug, PartialEq)]
pub struct RunConfig {
    pub name: String,
    /// A shell command line.
    pub command: String,
    /// Absolute.
    pub cwd: PathBuf,
    pub env: Vec<(String, String)>,
    pub source: Source,
    /// What to debug, when the file says.
    pub debug: Option<DebugProgram>,
    /// The debugger to use, by key, when the file says.
    pub debugger: Option<String>,
}

/// An executable to debug, from `program`, `args` and `build` in the file.
#[derive(Clone, Debug, PartialEq)]
pub struct DebugProgram {
    /// Absolute.
    pub program: PathBuf,
    pub args: Vec<String>,
    /// A shell command to build it with first.
    pub build: Option<String>,
}

/// Where a configuration came from, shown beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    File,
    Cargo,
    Npm,
    Go,
    Maven,
    Gradle,
}

impl Source {
    pub fn label(self) -> &'static str {
        match self {
            Source::File => "run.toml",
            Source::Cargo => "Cargo",
            Source::Npm => "package.json",
            Source::Go => "Go",
            Source::Maven => "Maven",
            Source::Gradle => "Gradle",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunFile {
    #[serde(default)]
    run: Vec<FileEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FileEntry {
    name: String,
    command: String,
    cwd: Option<PathBuf>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    program: Option<PathBuf>,
    #[serde(default)]
    args: Vec<String>,
    build: Option<String>,
    debugger: Option<String>,
}

/// Every configuration for the project at `root`: its file's first, then
/// the detected ones it doesn't replace. Runs `cargo metadata`, so call it
/// off the main thread. A broken file is an error alongside what was found.
pub fn load(root: &Path) -> (Vec<RunConfig>, Option<anyhow::Error>) {
    let (mut configs, error) = match from_file(root) {
        Ok(configs) => (configs, None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let named: HashSet<String> = configs.iter().map(|c| c.name.clone()).collect();
    configs.extend(
        detect(root)
            .into_iter()
            .filter(|config| !named.contains(&config.name)),
    );
    (configs, error)
}

/// The configurations in `root`'s `.jig/run.toml`; none when there is no
/// file.
pub fn from_file(root: &Path) -> Result<Vec<RunConfig>> {
    let path = root.join(FILE);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("Couldn't read {FILE}")),
    };
    let file: RunFile = toml::from_str(&text).with_context(|| format!("{FILE} is not valid"))?;
    Ok(file
        .run
        .into_iter()
        .filter(|entry| !entry.command.trim().is_empty())
        .map(|entry| RunConfig {
            name: entry.name,
            command: entry.command,
            cwd: entry
                .cwd
                .map_or_else(|| root.to_path_buf(), |cwd| root.join(cwd)),
            env: entry.env.into_iter().collect(),
            source: Source::File,
            debug: entry.program.map(|program| DebugProgram {
                program: root.join(program),
                args: entry.args,
                build: entry.build.filter(|build| !build.trim().is_empty()),
            }),
            debugger: entry.debugger,
        })
        .collect())
}

/// What the project declares it can run.
pub fn detect(root: &Path) -> Vec<RunConfig> {
    let mut configs = Vec::new();
    if root.join("Cargo.toml").is_file() {
        configs.extend(cargo(root));
    }
    if root.join("package.json").is_file() {
        configs.extend(npm(root));
    }
    if root.join("go.mod").is_file() {
        configs.extend(go(root));
    }
    if root.join("pom.xml").is_file() {
        configs.extend(maven(root));
    }
    if ["build.gradle", "build.gradle.kts"]
        .iter()
        .any(|file| root.join(file).is_file())
    {
        configs.extend(gradle(root));
    }
    dedupe_names(&mut configs);
    configs
}

fn config(root: &Path, name: String, command: String, source: Source) -> RunConfig {
    RunConfig {
        name,
        command,
        cwd: root.to_path_buf(),
        env: Vec::new(),
        source,
        debug: None,
        debugger: None,
    }
}

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    targets: Vec<Target>,
}

#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
}

/// Each binary and example, then the tests and a build. Asks Cargo, which
/// knows the workspace's members and the targets it infers from `src/bin`.
fn cargo(root: &Path) -> Vec<RunConfig> {
    let output = Command::new(find_program("cargo").unwrap_or_else(|| "cargo".into()))
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .env("PATH", crate::lsp::search_path())
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    let Some(metadata) = output
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| serde_json::from_slice::<Metadata>(&output.stdout).ok())
    else {
        return Vec::new();
    };
    cargo_configs(root, &metadata)
}

fn cargo_configs(root: &Path, metadata: &Metadata) -> Vec<RunConfig> {
    let mut configs = Vec::new();
    for kind in ["bin", "example"] {
        for package in &metadata.packages {
            for target in package
                .targets
                .iter()
                .filter(|target| target.kind.iter().any(|k| k == kind))
            {
                let (name, flag) = match kind {
                    "bin" => (target.name.clone(), "--bin"),
                    _ => (format!("{} (example)", target.name), "--example"),
                };
                let command = format!(
                    "cargo run -p {} {flag} {}",
                    shell_word(&package.name),
                    shell_word(&target.name)
                );
                configs.push(config(root, name, command, Source::Cargo));
            }
        }
    }
    configs.push(config(
        root,
        "Test".into(),
        "cargo test".into(),
        Source::Cargo,
    ));
    configs.push(config(
        root,
        "Build".into(),
        "cargo build".into(),
        Source::Cargo,
    ));
    configs
}

/// Each script in `package.json`, run with the package manager whose
/// lockfile is there.
fn npm(root: &Path) -> Vec<RunConfig> {
    let Ok(text) = std::fs::read_to_string(root.join("package.json")) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let Some(scripts) = json.get("scripts").and_then(|s| s.as_object()) else {
        return Vec::new();
    };
    let manager = [
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
    ]
    .into_iter()
    .find(|(lockfile, _)| root.join(lockfile).is_file())
    .map_or("npm", |(_, manager)| manager);
    scripts
        .keys()
        .map(|script| {
            let command = format!("{manager} run {}", shell_word(script));
            config(root, script.clone(), command, Source::Npm)
        })
        .collect()
}

fn go(root: &Path) -> Vec<RunConfig> {
    let mut configs = Vec::new();
    if root.join("main.go").is_file() {
        configs.push(config(root, "go run".into(), "go run .".into(), Source::Go));
    }
    configs.push(config(
        root,
        "go test".into(),
        "go test ./...".into(),
        Source::Go,
    ));
    configs
}

/// Each class under `src/main/java` with a `main` method, run with Maven's
/// exec plugin. Debugging takes that class's file, so it debugs that one.
fn maven(root: &Path) -> Vec<RunConfig> {
    let mvn = if root.join("mvnw").is_file() {
        "./mvnw"
    } else {
        "mvn"
    };
    main_classes(&root.join("src/main/java"))
        .into_iter()
        .map(|(class, file)| {
            let name = class.rsplit('.').next().unwrap_or(&class).to_string();
            let command = format!(
                "{mvn} -q compile exec:java -Dexec.mainClass={}",
                shell_word(&class)
            );
            RunConfig {
                debug: Some(DebugProgram {
                    program: file,
                    args: Vec::new(),
                    build: None,
                }),
                ..config(root, name, command, Source::Maven)
            }
        })
        .collect()
}

/// Gradle's `run`, for projects with the application plugin.
fn gradle(root: &Path) -> Vec<RunConfig> {
    let gradle = if root.join("gradlew").is_file() {
        "./gradlew"
    } else {
        "gradle"
    };
    vec![config(
        root,
        "gradle run".into(),
        format!("{gradle} run"),
        Source::Gradle,
    )]
}

/// Classes under `dir` with a `main` method, as (qualified name, file).
/// Looks no further than a few thousand files.
fn main_classes(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut found = Vec::new();
    let walk = ignore::WalkBuilder::new(dir).build();
    for entry in walk.filter_map(Result::ok).take(5_000) {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "java") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        if !text.contains("static void main(") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let package = text.lines().find_map(|line| {
            line.trim()
                .strip_prefix("package ")
                .map(|rest| rest.trim_end_matches(';').trim().to_string())
        });
        let class = match package {
            Some(package) => format!("{package}.{stem}"),
            None => stem.to_string(),
        };
        found.push((class, path.to_path_buf()));
    }
    found.sort();
    found
}

/// Detected names can clash, e.g. a `test` script and Cargo's tests; the
/// later ones get their source added.
fn dedupe_names(configs: &mut [RunConfig]) {
    let mut seen = HashSet::new();
    for config in configs {
        if !seen.insert(config.name.to_lowercase()) {
            config.name = format!("{} ({})", config.name, config.source.label());
            seen.insert(config.name.to_lowercase());
        }
    }
}

/// `command` split into words as `sh` would, or `None` when it does more
/// than run one program: pipes, `&&`, variables, redirections.
pub fn split_command(command: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        c => word.push(c),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => word.push(chars.next()?),
                        '$' | '`' => return None,
                        c => word.push(c),
                    }
                }
            }
            '\\' => {
                in_word = true;
                word.push(chars.next()?);
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '|' | '&' | ';' | '<' | '>' | '$' | '`' | '(' | ')' | '*' | '?' => return None,
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    (!words.is_empty()).then_some(words)
}

/// `word` quoted for `sh` if it needs it.
pub fn shell_word(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.:/@+=,".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

fn find_program(name: &str) -> Option<PathBuf> {
    std::env::split_paths(crate::lsp::search_path())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn reads_the_file_relative_to_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join(".jig")).unwrap();
        fs::write(
            root.join(FILE),
            r#"
[[run]]
name = "Serve"
command = "python -m http.server"
cwd = "site"
env = { PORT = "8000" }

[[run]]
name = "Blank"
command = "  "
"#,
        )
        .unwrap();
        let configs = from_file(root).unwrap();
        assert_eq!(
            configs,
            [RunConfig {
                name: "Serve".into(),
                command: "python -m http.server".into(),
                cwd: root.join("site"),
                env: vec![("PORT".into(), "8000".into())],
                source: Source::File,
                debug: None,
                debugger: None,
            }]
        );
    }

    #[test]
    fn a_missing_file_is_no_configurations_and_a_broken_one_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(from_file(dir.path()).unwrap().is_empty());
        fs::create_dir_all(dir.path().join(".jig")).unwrap();
        fs::write(dir.path().join(FILE), "[[run]]\nname = 1").unwrap();
        assert!(from_file(dir.path()).is_err());
    }

    #[test]
    fn the_template_parses_once_filled_in() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join(".jig")).unwrap();
        fs::write(
            dir.path().join(FILE),
            TEMPLATE.replace("command = \"\"", "command = \"make\""),
        )
        .unwrap();
        assert_eq!(from_file(dir.path()).unwrap()[0].command, "make");
    }

    #[test]
    fn the_file_replaces_detected_configurations_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(
            root.join("package.json"),
            r#"{"scripts": {"dev": "vite", "test": "vitest"}}"#,
        )
        .unwrap();
        fs::write(root.join("pnpm-lock.yaml"), "").unwrap();
        fs::create_dir_all(root.join(".jig")).unwrap();
        fs::write(
            root.join(FILE),
            "[[run]]\nname = \"dev\"\ncommand = \"pnpm dev --open\"\n",
        )
        .unwrap();
        let (configs, error) = load(root);
        assert!(error.is_none());
        let commands: Vec<_> = configs.iter().map(|c| c.command.as_str()).collect();
        assert_eq!(commands, ["pnpm dev --open", "pnpm run test"]);
    }

    #[test]
    fn cargo_lists_binaries_then_examples_then_test_and_build() {
        let metadata: Metadata = serde_json::from_str(
            r#"{"packages": [
                {"name": "jig-app", "targets": [{"name": "Jig", "kind": ["bin"]}]},
                {"name": "jig-ai", "targets": [
                    {"name": "jig_ai", "kind": ["lib"]},
                    {"name": "live", "kind": ["example"]}
                ]}
            ]}"#,
        )
        .unwrap();
        let configs = cargo_configs(Path::new("/p"), &metadata);
        let listed: Vec<_> = configs
            .iter()
            .map(|c| (c.name.as_str(), c.command.as_str()))
            .collect();
        assert_eq!(
            listed,
            [
                ("Jig", "cargo run -p jig-app --bin Jig"),
                ("live (example)", "cargo run -p jig-ai --example live"),
                ("Test", "cargo test"),
                ("Build", "cargo build"),
            ]
        );
    }

    #[test]
    fn maven_projects_list_their_main_classes() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("pom.xml"), "<project/>").unwrap();
        let sources = root.join("src/main/java/sample");
        fs::create_dir_all(&sources).unwrap();
        fs::write(
            sources.join("App.java"),
            "package sample;\npublic class App {\n  public static void main(String[] args) {}\n}\n",
        )
        .unwrap();
        fs::write(
            sources.join("Util.java"),
            "package sample;\nclass Util {}\n",
        )
        .unwrap();
        let configs = detect(root);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].name, "App");
        assert_eq!(
            configs[0].command,
            "mvn -q compile exec:java -Dexec.mainClass=sample.App"
        );
        assert_eq!(
            configs[0].debug.as_ref().unwrap().program,
            sources.join("App.java")
        );
    }

    #[test]
    fn clashing_detected_names_get_their_source() {
        let mut configs = vec![
            config(
                Path::new("/p"),
                "Test".into(),
                "cargo test".into(),
                Source::Cargo,
            ),
            config(
                Path::new("/p"),
                "test".into(),
                "npm run test".into(),
                Source::Npm,
            ),
        ];
        dedupe_names(&mut configs);
        assert_eq!(configs[1].name, "test (package.json)");
    }

    #[test]
    fn commands_split_like_sh() {
        assert_eq!(
            split_command(r#"cargo run -- "two words" 'it''s' a\ b"#).unwrap(),
            ["cargo", "run", "--", "two words", "its", "a b"]
        );
        assert_eq!(split_command("echo $HOME"), None);
        assert_eq!(split_command("ls | wc"), None);
        assert_eq!(split_command("echo 'open"), None);
    }

    #[test]
    fn words_are_quoted_only_when_needed() {
        assert_eq!(shell_word("build:prod"), "build:prod");
        assert_eq!(shell_word("it's here"), r"'it'\''s here'");
    }
}
