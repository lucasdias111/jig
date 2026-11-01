use super::*;
use crate::domain::languages::load;
use crate::ui::{gdscript, grammars};

#[test]
fn settings_start_with_bundled_languages() {
    let starters = jig_settings::LanguageSettings::default().added;
    assert!(starters.contains(&"rust".to_string()));
    for name in &starters {
        assert!(
            get(name).is_some(),
            "{name} is a starter but not a language"
        );
    }
}

#[test]
fn defaults_cover_the_bundled_languages() {
    let settings = LanguageSettings::default();
    let lang = |path: &str| language_for(Path::new(path), &settings);
    assert_eq!(lang("src/main.rs"), "rust");
    assert_eq!(lang("app.ts"), "typescript");
    assert_eq!(lang("App.tsx"), "tsx");
    assert_eq!(lang("index.mjs"), "javascript");
    assert_eq!(lang("Main.java"), "java");
    assert_eq!(lang("NOTES.MD"), "markdown");
    assert_eq!(lang("player.gd"), "gdscript");
    assert_eq!(lang("Makefile"), "text");
    assert_eq!(lang("a.unknown"), "text");
}

#[test]
fn settings_turn_off_and_remap() {
    let mut settings = LanguageSettings::default();
    settings.set_off("python", true);
    settings
        .extensions
        .insert("typescript".into(), ".TS, mts tsx".into());
    let lang = |path: &str| language_for(Path::new(path), &settings);
    assert_eq!(lang("a.py"), "text");
    assert_eq!(lang("a.mts"), "typescript");
    assert_eq!(lang("a.cts"), "text", "no longer listed");
    assert_eq!(lang("a.tsx"), "typescript", "TypeScript comes first");
    assert_eq!(
        written_in(Path::new("a.py"), &settings).map(|l| l.name),
        Some("python"),
        "still Python, though not highlighted"
    );
}

#[test]
fn languages_highlight_once_added() {
    let mut settings = LanguageSettings::default();
    let lang = |path: &str, settings: &LanguageSettings| language_for(Path::new(path), settings);
    assert_eq!(lang("app.rb", &settings), "text", "Ruby isn't a starter");
    assert!(written_in(Path::new("app.rb"), &settings).is_none());
    settings.set_added("ruby", true);
    assert_eq!(lang("app.rb", &settings), "ruby");
    assert_eq!(lang("util.h", &settings), "text");
    settings.set_added("cpp", true);
    assert_eq!(lang("util.h", &settings), "cpp");
    settings.set_added("c", true);
    assert_eq!(lang("util.h", &settings), "c", "C comes first");
    settings.set_added("rust", false);
    assert_eq!(lang("main.rs", &settings), "text");
}

#[test]
fn whole_file_names_pick_a_language() {
    let mut settings = LanguageSettings::default();
    settings.set_added("make", true);
    let lang = |path: &str| language_for(Path::new(path), &settings);
    assert_eq!(lang("src/Makefile"), "make");
    assert_eq!(lang("rules.mk"), "make");
    assert_eq!(lang("Makefile.old"), "text", "only the whole name");
}

#[test]
fn the_user_s_servers_replace_and_come_first() {
    let table = load(Some(
        r#"
            [[server]]
            name = "pylsp"
            languages = ["python"]
            commands = [["pylsp"]]

            [[server]]
            name = "gopls"
            languages = ["go"]
            commands = [["my-gopls"]]
            "#,
    ));
    assert!(table.user_error.is_none());
    let first = |language: &str| {
        table
            .servers
            .iter()
            .find(|s| s.languages.iter().any(|l| l == language))
            .map(|s| (s.name.as_str(), s.commands[0][0].as_str()))
    };
    assert_eq!(first("python"), Some(("pylsp", "pylsp")));
    assert_eq!(first("go"), Some(("gopls", "my-gopls")));
    assert_eq!(
        table.servers.iter().filter(|s| s.name == "gopls").count(),
        1
    );
    assert_eq!(first("rust"), Some(("rust-analyzer", "rust-analyzer")));
}

#[test]
fn a_bad_user_file_says_so_and_keeps_the_built_ins() {
    let table = load(Some("[[server]]\nnmae = \"x\"\n"));
    assert!(
        table
            .user_error
            .unwrap()
            .starts_with("language-servers.toml isn't valid")
    );
    assert_eq!(table.servers.len(), load(None).servers.len());
    assert!(load(Some(USER_TEMPLATE)).user_error.is_none());
}

#[test]
fn the_table_hangs_together() {
    let mut names = std::collections::HashSet::new();
    for language in bundled() {
        assert!(names.insert(language.name), "{} twice", language.name);
        assert!(
            !language.extensions.is_empty() || !language.filenames.is_empty(),
            "{} takes no files",
            language.name
        );
    }
    let mut server_names = std::collections::HashSet::new();
    for server in servers() {
        assert!(server_names.insert(&server.name), "{} twice", server.name);
        assert!(
            !server.commands.is_empty(),
            "{} has no command",
            server.name
        );
        for language in &server.languages {
            assert!(
                get(language).is_some(),
                "{} serves unknown {language}",
                server.name
            );
        }
    }
    // One server per language: the first would always win.
    for language in bundled() {
        let serving = servers()
            .iter()
            .filter(|s| s.languages.iter().any(|l| l == language.name))
            .count();
        assert!(serving <= 1, "{} has {serving} servers", language.name);
    }
}

#[test]
fn every_bundled_grammar_is_compiled_in() {
    gdscript::register();
    grammars::register();
    let registry = gpui_kit::component::highlighter::LanguageRegistry::singleton();
    for language in bundled() {
        assert!(
            registry
                .language(language.name)
                .is_some_and(|config| config.has_grammar()),
            "{} has no grammar; is its gpui-kit feature on?",
            language.name
        );
    }
}
