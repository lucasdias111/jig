use std::fs;

use super::*;
use std::path::Path;

use crate::domain::run_configs::{Metadata, cargo_configs, config, dedupe_names};

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
