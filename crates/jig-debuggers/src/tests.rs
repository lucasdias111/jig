use std::fs;
use std::path::Path;

use serde_json::json;

use super::*;
use crate::domain::debuggers::parse;
use crate::store::cache::load;
use crate::store::persist::{glob, install_into, installed_in};

#[test]
fn every_debugger_names_known_languages() {
    for debugger in &registry().debuggers {
        for language in &debugger.languages {
            assert!(
                jig_languages::get(language).is_some(),
                "{} debugs unknown {language}",
                debugger.name
            );
        }
    }
}

#[test]
fn the_built_ins_parse_and_cover_five_languages() {
    let registry = load(None);
    assert!(registry.error.is_none());
    let keys: Vec<&str> = registry.debuggers.iter().map(|d| d.key.as_str()).collect();
    assert_eq!(keys, ["rust", "typescript", "python", "go", "java"]);
    assert_eq!(
        registry.get("java").unwrap().lsp(),
        Some(("jdtls", "vscode.java.startDebugSession"))
    );
    assert_eq!(registry.for_language("tsx").unwrap().key, "typescript");
    assert!(registry.for_language("toml").is_none());
}

#[test]
fn uninstall_only_reaches_into_the_debuggers_folder() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("bin")).unwrap();
    fs::write(dir.path().join("bin/dlv"), "").unwrap();
    let debugger = parse(
        r#"
[[debugger]]
key = "go"
name = "Go"
adapter = ["dlv"]
uninstall = ["bin/dlv", "missing", "../outside", "/etc/hosts"]
[debugger.launch]
"#,
    )
    .unwrap()
    .remove(0);
    assert_eq!(
        installed_in(&debugger, dir.path()),
        [dir.path().join("bin/dlv")]
    );
}

#[test]
fn the_users_file_replaces_and_adds() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("debuggers.toml");
    fs::write(
        &path,
        r#"
[[debugger]]
key = "python"
name = "My Python"
adapter = ["mypy-dap"]
[debugger.launch]
request = "launch"

[[debugger]]
key = "ruby"
name = "Ruby"
languages = ["ruby"]
server = ["rdbg", "--port", "${port}"]
[debugger.launch]
request = "launch"
"#,
    )
    .unwrap();
    let registry = load(Some(&path));
    assert!(registry.error.is_none(), "{:?}", registry.error);
    assert_eq!(registry.get("python").unwrap().name, "My Python");
    assert_eq!(registry.get("ruby").unwrap().adapter_id(), "ruby");
    assert_eq!(registry.debuggers.len(), 6);

    fs::write(
        &path,
        "[[debugger]]\nkey = \"x\"\nname = \"X\"\n[debugger.launch]\n",
    )
    .unwrap();
    let broken = load(Some(&path));
    assert!(broken.error.unwrap().contains("adapter"));
    assert_eq!(broken.debuggers.len(), 5, "the built-ins stay");
}

#[test]
fn variables_fill_in_text_and_keep_their_type_when_whole() {
    let mut vars = Vars::new(Path::new("/p"));
    vars.set("program", json!("/p/app.py"))
        .set("args", json!(["-v", "two words"]))
        .set("env", json!({"PORT": "8080"}));
    let template = json!({
        "program": "${program}",
        "args": "${args}",
        "env": "${env}",
        "title": "debugging ${program} now",
        "nested": ["${program}", {"a": "${args}"}],
        "plain": 3,
    });
    assert_eq!(
        vars.expand(&template).unwrap(),
        json!({
            "program": "/p/app.py",
            "args": ["-v", "two words"],
            "env": {"PORT": "8080"},
            "title": "debugging /p/app.py now",
            "nested": ["/p/app.py", {"a": ["-v", "two words"]}],
            "plain": 3,
        })
    );
    assert!(
        vars.expand(&json!("${nonsense}"))
            .unwrap_err()
            .contains("${nonsense}")
    );
}

#[test]
fn bundles_match_by_file_name() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["plugin-0.53.1.jar", "plugin-0.52.0.jar", "other.jar"] {
        fs::write(dir.path().join(name), "").unwrap();
    }
    let pattern = format!("{}/plugin-*.jar", dir.path().display());
    assert_eq!(
        glob(&pattern),
        [
            dir.path().join("plugin-0.52.0.jar"),
            dir.path().join("plugin-0.53.1.jar")
        ]
    );
    assert!(glob(&format!("{}/missing-*.jar", dir.path().display())).is_empty());
}

#[test]
fn help_follows_the_system() {
    let rust = load(None).get("rust").unwrap();
    let expected = if cfg!(target_os = "macos") {
        "xcode-select"
    } else {
        "package manager"
    };
    assert!(rust.help().contains(expected), "{}", rust.help());
}

#[test]
fn install_runs_the_command_in_the_debuggers_folder() {
    let dir = tempfile::tempdir().unwrap();
    let mut debugger = load(None).get("go").unwrap().as_ref().clone();
    debugger.install =
        Some(r#"mkdir -p "${debuggers}/bin" && touch "${debuggers}/bin/made""#.into());
    install_into(&debugger, dir.path()).unwrap();
    assert!(dir.path().join("bin/made").is_file());

    debugger.install = Some("echo nope >&2; exit 3".into());
    let error = install_into(&debugger, dir.path()).unwrap_err().to_string();
    assert!(error.contains("nope"), "{error}");
}

/// Runs every built-in install command into a temporary folder, as
/// Settings' Install button would. Downloads js-debug, debugpy, Delve,
/// java-debug and, unless it's installed, jdtls; Go's module cache goes
/// to the temporary folder too.
#[test]
#[ignore]
fn built_in_installers_live() {
    let dir = tempfile::tempdir().unwrap();
    // SAFETY: run on its own; nothing else reads these meanwhile.
    unsafe {
        std::env::set_var("GOPATH", dir.path().join("gopath"));
        std::env::set_var("GOFLAGS", "-modcacherw");
    }
    let debuggers = dir.path().join("debuggers");
    let registry = load(None);
    for (key, made) in [
        ("typescript", "js-debug/src/dapDebugServer.js"),
        ("python", "debugpy/debugpy/__init__.py"),
        ("go", "bin/dlv"),
        (
            "java",
            "java-debug/com.microsoft.java.debug.plugin-0.53.1.jar",
        ),
    ] {
        install_into(&registry.get(key).unwrap(), &debuggers)
            .unwrap_or_else(|e| panic!("{key}: {e:#}"));
        assert!(debuggers.join(made).is_file(), "{key} made no {made}");
    }
}
