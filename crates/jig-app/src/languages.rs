//! The languages Jig can highlight, and which file each one applies to.
//!
//! Grammars are compiled in (see the `gpui-kit` features in the workspace
//! `Cargo.toml`); Settings turns them on or off and changes their extensions.

use std::path::Path;

use crate::settings::LanguageSettings;

/// A language with a bundled tree-sitter grammar.
pub struct Language {
    /// The highlighter's name for it.
    pub name: &'static str,
    pub label: &'static str,
    /// File extensions it applies to unless Settings says otherwise.
    pub extensions: &'static [&'static str],
}

/// Every bundled language. Where two claim an extension, the first wins.
pub const BUNDLED: &[Language] = &[
    Language {
        name: "rust",
        label: "Rust",
        extensions: &["rs"],
    },
    Language {
        name: "typescript",
        label: "TypeScript",
        extensions: &["ts", "mts", "cts"],
    },
    Language {
        name: "tsx",
        label: "TSX",
        extensions: &["tsx"],
    },
    Language {
        name: "javascript",
        label: "JavaScript",
        extensions: &["js", "mjs", "cjs", "jsx"],
    },
    Language {
        name: "java",
        label: "Java",
        extensions: &["java"],
    },
    Language {
        name: "python",
        label: "Python",
        extensions: &["py", "pyi"],
    },
    Language {
        name: "go",
        label: "Go",
        extensions: &["go"],
    },
    Language {
        name: "toml",
        label: "TOML",
        extensions: &["toml"],
    },
    Language {
        name: "json",
        label: "JSON",
        extensions: &["json", "jsonc"],
    },
    Language {
        name: "markdown",
        label: "Markdown",
        extensions: &["md", "markdown"],
    },
];

/// Extensions as Settings shows them: `"ts, mts, cts"`.
pub fn format_extensions(extensions: &[&str]) -> String {
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
pub fn written_in(path: &Path, settings: &LanguageSettings) -> Option<&'static Language> {
    matching(path, settings).next()
}

/// The languages claiming `path`'s extension, first first.
fn matching<'a>(
    path: &Path,
    settings: &'a LanguageSettings,
) -> impl Iterator<Item = &'static Language> + 'a {
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase);
    BUNDLED.iter().filter(move |language| {
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
    fn every_bundled_grammar_is_compiled_in() {
        let registry = gpui_kit::component::highlighter::LanguageRegistry::singleton();
        for language in BUNDLED {
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
