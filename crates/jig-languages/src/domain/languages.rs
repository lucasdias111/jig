//! The languages and servers as `assets/default-languages.toml` lists them,
//! with the user's servers merged in, and which language a file is in.

use std::path::Path;

use serde::Deserialize;

use jig_settings::LanguageSettings;

use super::servers::Server;

const BUILT_IN: &str = include_str!("../../../../assets/default-languages.toml");

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
    // Read by `settings`, which starts every list with the starters.
    #[serde(default)]
    #[allow(dead_code)]
    starter: bool,
    line_comment: Option<String>,
    lsp_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UserFile {
    #[serde(default)]
    server: Vec<Server>,
}

/// The languages and servers, with the user's servers, and what was wrong
/// with the user's file if anything. Kept for the life of the app, so its
/// text is handed out as `&'static str`.
pub struct Table {
    pub languages: Vec<Language>,
    pub servers: Vec<Server>,
    pub user_error: Option<String>,
}

/// The built-in table with `user`, the text of the user's
/// `language-servers.toml`, merged in.
pub fn load(user: Option<&str>) -> Table {
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

/// The language of `languages` to highlight `path` with, or `"text"` for
/// none.
pub fn language_for(
    languages: &'static [Language],
    path: &Path,
    settings: &LanguageSettings,
) -> &'static str {
    matching(languages, path, settings)
        .find(|language| !settings.is_off(language.name))
        .map_or("text", |language| language.name)
}

/// The language `path` is written in, whether or not it's highlighted.
/// `None` for a language that isn't added.
pub fn written_in(
    languages: &'static [Language],
    path: &Path,
    settings: &LanguageSettings,
) -> Option<&'static Language> {
    matching(languages, path, settings).next()
}

/// The added languages claiming `path`'s extension, first first.
fn matching<'a>(
    languages: &'static [Language],
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
    languages.iter().filter(move |language| {
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
