//! The languages Jig can highlight, and which file each one applies to:
//! `assets/default-languages.toml`, which also lists the language servers.
//!
//! Grammars are compiled in (see the `gpui-kit` features in the workspace
//! `Cargo.toml`); Settings adds languages to the list, turns them on or off
//! and changes their extensions.

pub mod gdscript;

use std::path::Path;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::lsp::Server;
use crate::settings::LanguageSettings;

const BUILT_IN: &str = include_str!("../../../assets/default-languages.toml");

/// A language with a bundled tree-sitter grammar.
#[derive(Debug)]
pub struct Language {
    /// The highlighter's name for it.
    pub name: &'static str,
    pub label: &'static str,
    /// File extensions it applies to unless Settings says otherwise.
    pub extensions: Vec<&'static str>,
    /// Whole file names it applies to, for files like `Makefile` that
    /// have no extension to go by.
    pub filenames: Vec<&'static str>,
    /// Whether it's in the list from the start; the rest are added from
    /// Settings.
    pub starter: bool,
    /// What starts a line comment, if it has them.
    pub line_comment: Option<&'static str>,
    /// The protocol's languageId.
    pub lsp_id: &'static str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LanguageFile {
    language: Vec<LanguageEntry>,
    server: Vec<Server>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LanguageEntry {
    name: String,
    label: String,
    extensions: Vec<String>,
    #[serde(default)]
    filenames: Vec<String>,
    #[serde(default)]
    starter: bool,
    line_comment: Option<String>,
    lsp_id: Option<String>,
}

/// A starter for the user's `language-servers.toml`.
pub const USER_TEMPLATE: &str = r#"# Your language servers. One with the same name as a built-in replaces it;
# others are added, and a server here for a language wins over the built-in
# one. Restart Jig after changing this file. The fields are the ones in
# assets/default-languages.toml in Jig's source, which lists the built-ins.
#
# [[server]]
# name = "pylsp"
# languages = ["python"]
# commands = [["pylsp"]]
# root_markers = ["pyproject.toml", "setup.py"]
#
# A server with no commands turns a language's server off:
#
# [[server]]
# name = "none"
# languages = ["markdown"]
# commands = []
"#;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UserFile {
    #[serde(default)]
    server: Vec<Server>,
}

/// The languages and servers, read once at startup with the user's
/// servers, and what was wrong with the user's file if anything. Kept for
/// the life of the app, so its text is handed out as `&'static str`.
struct Table {
    languages: Vec<Language>,
    servers: Vec<Server>,
    user_error: Option<String>,
}

static TABLE: LazyLock<Table> = LazyLock::new(|| {
    let user = user_path().and_then(|path| std::fs::read_to_string(path).ok());
    load(user.as_deref())
});

fn load(user: Option<&str>) -> Table {
    let file: LanguageFile = toml::from_str(BUILT_IN).expect("default-languages.toml parses");
    let leak = |text: String| -> &'static str { Box::leak(text.into_boxed_str()) };
    let languages = file
        .language
        .into_iter()
        .map(|entry| {
            let name = leak(entry.name);
            Language {
                name,
                label: leak(entry.label),
                extensions: entry.extensions.into_iter().map(leak).collect(),
                filenames: entry.filenames.into_iter().map(leak).collect(),
                starter: entry.starter,
                line_comment: entry.line_comment.map(leak),
                lsp_id: entry.lsp_id.map_or(name, leak),
            }
        })
        .collect();
    let mut servers = file.server;
    let mut user_error = None;
    match user.map(toml::from_str::<UserFile>) {
        None => {}
        Some(Ok(own)) => {
            // The user's come first, so theirs for a language wins.
            servers.retain(|built_in| own.server.iter().all(|s| s.name != built_in.name));
            servers.splice(0..0, own.server);
        }
        Some(Err(error)) => {
            user_error = Some(format!(
                "language-servers.toml isn't valid: {}",
                error.message()
            ))
        }
    }
    Table {
        languages,
        servers,
        user_error,
    }
}

/// The user's `language-servers.toml`; `None` in tests.
pub fn user_path() -> Option<std::path::PathBuf> {
    if cfg!(test) {
        return None;
    }
    crate::formatters::user_path().map(|path| path.with_file_name("language-servers.toml"))
}

/// Every bundled language: the starters, then the ones to add from
/// Settings. Where two claim an extension, the first wins.
pub fn bundled() -> &'static [Language] {
    &TABLE.languages
}

/// Every language server Jig knows how to run, the user's first.
pub fn servers() -> &'static [Server] {
    &TABLE.servers
}

/// What was wrong with the user's `language-servers.toml` at startup.
pub fn user_error() -> Option<&'static str> {
    TABLE.user_error.as_deref()
}

/// A bundled language by name.
pub fn get(name: &str) -> Option<&'static Language> {
    bundled().iter().find(|language| language.name == name)
}

/// What starts a line comment in `language`, Jig's name for it. `None` for
/// languages without one, such as JSON.
pub fn line_comment(language: &str) -> Option<&'static str> {
    get(language)?.line_comment
}

/// Extensions as Settings shows them: `"ts, mts, cts"`.
pub fn format_extensions(extensions: &[&'static str]) -> String {
    extensions.join(", ")
}

/// Extensions as typed in Settings: separated by commas or spaces, with or
/// without a leading dot, in any case.
pub fn parse_extensions(text: &str) -> Vec<String> {
    text.split([',', ' '])
        .map(|ext| ext.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|ext| !ext.is_empty())
        .collect()
}

/// The language to highlight `path` with, or `"text"` for none.
pub fn language_for(path: &Path, settings: &LanguageSettings) -> &'static str {
    matching(path, settings)
        .find(|language| !settings.is_off(language.name))
        .map_or("text", |language| language.name)
}

/// The language `path` is written in, whether or not it's highlighted.
/// `None` for a language that isn't added.
pub fn written_in(path: &Path, settings: &LanguageSettings) -> Option<&'static Language> {
    matching(path, settings).next()
}

/// The added languages claiming `path`'s extension, first first.
fn matching<'a>(
    path: &Path,
    settings: &'a LanguageSettings,
) -> impl Iterator<Item = &'static Language> + 'a {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string);
    bundled().iter().filter(move |language| {
        if !settings.is_added(language.name) {
            return false;
        }
        if file_name
            .as_deref()
            .is_some_and(|name| language.filenames.contains(&name))
        {
            return true;
        }
        let Some(ext) = ext.as_deref() else {
            return false;
        };
        match settings.extensions.get(language.name) {
            Some(text) => parse_extensions(text).iter().any(|own| own == ext),
            None => language.extensions.contains(&ext),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let lang =
            |path: &str, settings: &LanguageSettings| language_for(Path::new(path), settings);
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
        // Formatters and debuggers name languages too.
        for formatter in &crate::formatters::registry().formatters {
            for language in &formatter.languages {
                assert!(
                    get(language).is_some(),
                    "{} formats unknown {language}",
                    formatter.name
                );
            }
        }
        for debugger in &crate::debuggers::registry().debuggers {
            for language in &debugger.languages {
                assert!(
                    get(language).is_some(),
                    "{} debugs unknown {language}",
                    debugger.name
                );
            }
        }
    }

    #[test]
    fn every_bundled_grammar_is_compiled_in() {
        gdscript::register();
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
}
