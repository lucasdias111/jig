use std::fs;

use super::*;

#[test]
fn every_formatter_names_known_languages() {
    for formatter in &registry().formatters {
        for language in &formatter.languages {
            assert!(
                jig_languages::get(language).is_some(),
                "{} formats unknown {language}",
                formatter.name
            );
        }
    }
}

const LIMIT: Duration = Duration::from_secs(10);

#[test]
fn the_built_ins_parse() {
    let registry = load(None);
    assert!(registry.error.is_none());
    let python: Vec<&str> = registry
        .formatters
        .iter()
        .filter(|f| f.languages.iter().any(|l| l == "python"))
        .map(|f| f.key.as_str())
        .collect();
    assert_eq!(python, ["ruff", "black"], "Ruff first");
    assert!(registry.knows("rust").is_some());
    assert!(registry.knows("markdown").is_some());
}

#[test]
fn the_users_file_replaces_adds_and_comes_first() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("formatters.toml");
    fs::write(
        &path,
        r#"
[[formatter]]
key = "rustfmt"
name = "My rustfmt"
languages = ["rust"]
command = ["rustfmt", "--edition", "2024"]

[[formatter]]
key = "yapf"
name = "YAPF"
languages = ["python"]
command = ["yapf"]
"#,
    )
    .unwrap();
    let registry = load(Some(&path));
    assert!(registry.error.is_none(), "{:?}", registry.error);
    assert_eq!(registry.knows("rust").unwrap().name, "My rustfmt");
    assert_eq!(registry.knows("python").unwrap().key, "yapf");
    assert_eq!(registry.formatters.len(), load(None).formatters.len() + 1);

    fs::write(
        &path,
        "[[formatter]]\nkey = \"x\"\nname = \"X\"\nlanguages = []\ncommand = []\n",
    )
    .unwrap();
    let broken = load(Some(&path));
    assert!(broken.error.unwrap().contains("command"));
    assert_eq!(broken.formatters.len(), load(None).formatters.len());
}

#[test]
fn formatters_that_arent_installed_are_skipped() {
    // Cargo is running these tests, so it's installed.
    let registry = Registry {
        formatters: parse(
            r#"
[[formatter]]
key = "missing"
name = "Missing"
languages = ["rust"]
command = ["jig-no-such-formatter"]

[[formatter]]
key = "cargo"
name = "Cargo"
languages = ["rust"]
command = ["cargo", "fmt"]
"#,
        )
        .unwrap()
        .into_iter()
        .map(Arc::new)
        .collect(),
        error: None,
    };
    let file = Path::new("/p/src/main.rs");
    assert_eq!(registry.for_language("rust", file).unwrap().key, "cargo");
    assert!(registry.for_language("go", file).is_none());
    let missing = registry.knows("rust").unwrap();
    assert_eq!(
        missing.format("x", file, Path::new("/p"), LIMIT),
        Err("Missing isn't installed.".into())
    );
}

#[cfg(unix)]
fn fake(script: &str) -> Formatter {
    Formatter {
        key: "fake".into(),
        name: "Fake".into(),
        languages: vec!["rust".into()],
        command: vec!["/bin/sh".into(), "-c".into(), script.into()],
        install: None,
        help: None,
    }
}

#[cfg(unix)]
#[test]
fn a_fake_formatter_gets_stdin_and_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.rs");
    let formatter = Formatter {
        command: vec![
            "/bin/sh".into(),
            "-c".into(),
            r#"tr a-z A-Z; printf '%s\n' "$1""#.into(),
            "sh".into(),
            "${file}".into(),
        ],
        ..fake("")
    };
    let out = formatter.format("fn a() {}\n", &file, dir.path(), LIMIT);
    assert_eq!(out.unwrap(), format!("FN A() {{}}\n{}\n", file.display()));
    // A Windows file keeps its line endings.
    let out = fake("cat").format("a\r\nb\r\n", &file, dir.path(), LIMIT);
    assert_eq!(out.unwrap(), "a\r\nb\r\n");
    let out = fake("tr -d '\\r'").format("a\r\nb\r\n", &file, dir.path(), LIMIT);
    assert_eq!(out.unwrap(), "a\r\nb\r\n");
}

#[cfg(unix)]
#[test]
fn failures_say_why_and_never_empty_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("a.rs");
    let failing = fake("cat >/dev/null; echo 'error: expected `;`' >&2; exit 1");
    assert_eq!(
        failing.format("x", &file, dir.path(), LIMIT),
        Err("Fake: error: expected `;`".into())
    );
    let silent = fake("cat >/dev/null");
    assert_eq!(
        silent.format("x", &file, dir.path(), LIMIT),
        Err("Fake: returned nothing".into())
    );
    let slow = fake("sleep 5");
    let started = Instant::now();
    let out = slow.format("x", &file, dir.path(), Duration::from_millis(200));
    assert!(out.unwrap_err().contains("no answer"));
    assert!(started.elapsed() < Duration::from_secs(3), "stopped");
}

#[test]
fn tidy_trims_and_ends_with_a_newline() {
    let text = "a  \n\tb\t\r\n  \nc ";
    assert_eq!(tidy(text, true, false, None), "a\n\tb\r\n\nc");
    assert_eq!(tidy(text, true, true, None), "a\n\tb\r\n\nc\r\n");
    assert_eq!(tidy(text, false, true, None), "a  \n\tb\t\r\n  \nc \r\n");
    assert_eq!(tidy("x\n", false, true, None), "x\n");
    assert_eq!(tidy("", true, true, None), "", "an empty file stays empty");
    assert_eq!(tidy(text, false, false, None), text);
}

#[test]
fn tidy_makes_the_line_endings_as_asked() {
    let text = "a\r\nb\nc";
    assert_eq!(tidy(text, false, true, Some(LineEnding::Lf)), "a\nb\nc\n");
    assert_eq!(
        tidy(text, false, true, Some(LineEnding::CrLf)),
        "a\r\nb\r\nc\r\n"
    );
}

/// Installs every built-in formatter Jig can install, or those named
/// in `JIG_LIVE_TOOLS` (`shfmt,stylua`), into a scratch folder and
/// formats a sample with each: `cargo test -p jig-app
/// formatter_installers_live -- --ignored --nocapture`. Downloads a lot.
#[test]
#[ignore]
fn formatter_installers_live() {
    let only = std::env::var("JIG_LIVE_TOOLS").ok();
    let scratch = tempfile::tempdir().unwrap();
    // SAFETY: run on its own; nothing else reads it meanwhile.
    unsafe { std::env::set_var("JIG_FORMATTERS_DIR", scratch.path()) };
    let samples: &[(&str, &str, &str)] = &[
        ("prettier", "a.ts", "let  x=1"),
        ("ruff", "a.py", "x=1"),
        ("black", "a.py", "x=1"),
        ("google-java-format", "A.java", "class A{int x;}"),
        ("taplo", "a.toml", "a=1"),
        ("clang-format", "a.c", "int  x=1;"),
        ("shfmt", "a.sh", "if true;then echo;fi"),
        ("stylua", "a.lua", "local  x=1"),
        ("ktfmt", "a.kt", "val  x=1"),
        ("sql-formatter", "a.sql", "select a from b"),
        ("gdformat", "a.gd", "var  x=1"),
        ("elm-format", "A.elm", "module A exposing (..)\nx=1"),
    ];
    let registry = load(None);
    let mut failed = Vec::new();
    for formatter in registry.formatters.iter().filter(|f| f.installable()) {
        if only
            .as_deref()
            .is_some_and(|only| !only.split(',').any(|key| key == formatter.key))
        {
            continue;
        }
        let Some((_, name, text)) = samples.iter().find(|(key, ..)| *key == formatter.key) else {
            continue;
        };
        let file = scratch.path().join(name);
        let result = formatter
            .install()
            .map_err(|error| format!("{error:#}"))
            .and_then(|()| formatter.format(text, &file, scratch.path(), LIMIT));
        match result {
            Ok(out) if out != *text => eprintln!("ok   {}: {out:?}", formatter.key),
            Ok(_) => {
                eprintln!("FAIL {}: changed nothing", formatter.key);
                failed.push(formatter.key.clone());
            }
            Err(error) => {
                eprintln!("FAIL {}: {error}", formatter.key);
                failed.push(formatter.key.clone());
            }
        }
    }
    assert!(failed.is_empty(), "{failed:?}");
}
