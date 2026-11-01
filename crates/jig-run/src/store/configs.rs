//! Finding a project's run configurations on disk: its `.jig/run.toml`,
//! and what Cargo, `package.json`, Go, Maven and Gradle declare.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result};
use jig_process::NoWindow as _;

use crate::domain::run_configs::{
    DebugProgram, FILE, Metadata, RunConfig, Source, cargo_configs, config, dedupe_names,
    parse_file, shell_word,
};

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
    parse_file(root, &text)
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

/// Each binary and example, then the tests and a build. Asks Cargo, which
/// knows the workspace's members and the targets it infers from `src/bin`.
fn cargo(root: &Path) -> Vec<RunConfig> {
    let output = Command::new(find_program("cargo").unwrap_or_else(|| "cargo".into()))
        .no_window()
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(root)
        .env("PATH", jig_tools::search_path())
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

fn find_program(name: &str) -> Option<PathBuf> {
    std::env::split_paths(jig_tools::search_path())
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}
