//! The grammars GPUI Kit has no feature for, each a crate of its own,
//! registered with the highlighter at startup under Jig's name for the
//! language. Their highlights are the grammar's own (or, where the crate
//! doesn't export them, the file it ships, in `queries/`), with the older
//! Neovim capture names some use mapped onto ones the theme colours.

use std::sync::Once;

use gpui_kit::component::highlighter::{GrammarConfig, LanguageRegistry};
use tree_sitter::Language;

/// A grammar: Jig's name for its language, the parser, and its highlights.
struct Grammar {
    name: &'static str,
    language: fn() -> Language,
    highlights: &'static str,
}

macro_rules! grammar {
    ($name:literal, $language:expr, $highlights:expr) => {
        Grammar {
            name: $name,
            language: || $language.into(),
            highlights: $highlights,
        }
    };
}

/// Keep in step with `assets/default-languages.toml`.
const GRAMMARS: &[Grammar] = &[
    grammar!(
        "ada",
        tree_sitter_ada::LANGUAGE,
        include_str!("queries/ada.scm")
    ),
    grammar!("d", tree_sitter_d::LANGUAGE, include_str!("queries/d.scm")),
    grammar!(
        "dart",
        tree_sitter_dart::LANGUAGE,
        tree_sitter_dart::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "dockerfile",
        tree_sitter_dockerfile_updated::language(),
        include_str!("queries/dockerfile.scm")
    ),
    grammar!(
        "elisp",
        tree_sitter_elisp::LANGUAGE,
        tree_sitter_elisp::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "elm",
        tree_sitter_elm::LANGUAGE,
        tree_sitter_elm::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "erlang",
        tree_sitter_erlang::LANGUAGE,
        tree_sitter_erlang::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "fish",
        tree_sitter_fish::language(),
        tree_sitter_fish::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "gleam",
        tree_sitter_gleam::LANGUAGE,
        tree_sitter_gleam::HIGHLIGHT_QUERY
    ),
    grammar!(
        "glsl",
        tree_sitter_glsl::LANGUAGE_GLSL,
        tree_sitter_glsl::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "haskell",
        tree_sitter_haskell::LANGUAGE,
        tree_sitter_haskell::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "ini",
        tree_sitter_ini::LANGUAGE,
        tree_sitter_ini::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "julia",
        tree_sitter_julia::LANGUAGE,
        include_str!("queries/julia.scm")
    ),
    grammar!(
        "less",
        tree_sitter_less::language(),
        tree_sitter_less::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "matlab",
        tree_sitter_matlab::LANGUAGE,
        include_str!("queries/matlab.scm")
    ),
    grammar!(
        "nix",
        tree_sitter_nix::LANGUAGE,
        tree_sitter_nix::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "objc",
        tree_sitter_objc::LANGUAGE,
        tree_sitter_objc::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "odin",
        tree_sitter_odin::LANGUAGE,
        tree_sitter_odin::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "pascal",
        tree_sitter_pascal::LANGUAGE,
        include_str!("queries/pascal.scm")
    ),
    grammar!(
        "powershell",
        tree_sitter_powershell::LANGUAGE,
        tree_sitter_powershell::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "prisma",
        tree_sitter_prisma_io::LANGUAGE,
        include_str!("queries/prisma.scm")
    ),
    grammar!(
        "r",
        tree_sitter_r::LANGUAGE,
        tree_sitter_r::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "racket",
        tree_sitter_racket::LANGUAGE,
        tree_sitter_racket::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "scss",
        tree_sitter_scss::language(),
        tree_sitter_scss::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "starlark",
        tree_sitter_starlark::LANGUAGE,
        tree_sitter_starlark::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "vim",
        tree_sitter_vim::language(),
        tree_sitter_vim::HIGHLIGHTS_QUERY
    ),
    grammar!(
        "vue",
        tree_sitter_vue_next::LANGUAGE,
        include_str!("queries/vue.scm")
    ),
    grammar!(
        "xml",
        tree_sitter_xml::LANGUAGE_XML,
        tree_sitter_xml::XML_HIGHLIGHT_QUERY
    ),
];

/// Older Neovim capture names, by their first part, and the theme's name
/// for what they are.
const RENAMED: &[(&str, &str)] = &[
    ("character", "string"),
    ("conditional", "keyword"),
    ("escape", "string.escape"),
    ("exception", "keyword"),
    ("field", "property"),
    ("float", "number"),
    ("include", "keyword"),
    ("markup", "text.literal"),
    ("method", "function"),
    ("module", "type"),
    ("namespace", "type"),
    ("parameter", "variable"),
    ("repeat", "keyword"),
    ("storageclass", "keyword"),
];

/// `query` with its old capture names renamed. A capture is `@name`, the
/// name starting with a letter; an `@` inside a quoted string is text the
/// grammar matches, so it's left alone.
fn modernise(query: &str) -> String {
    let mut out = String::with_capacity(query.len());
    let mut in_string = false;
    let mut in_comment = false;
    let mut chars = query.char_indices().peekable();
    while let Some((ix, ch)) = chars.next() {
        match ch {
            '\n' => in_comment = false,
            ';' if !in_string => in_comment = true,
            '\\' if in_string => {
                out.push(ch);
                if let Some((_, next)) = chars.next() {
                    out.push(next);
                }
                continue;
            }
            '"' if !in_comment => in_string = !in_string,
            '@' if !in_string && !in_comment => {
                let rest = &query[ix + 1..];
                let len = rest
                    .find(|c: char| !(c.is_alphanumeric() || c == '_' || c == '.'))
                    .unwrap_or(rest.len());
                let name = &rest[..len];
                let (head, tail) = name.split_once('.').unwrap_or((name, ""));
                if let Some((_, theme)) = RENAMED.iter().find(|(old, _)| *old == head) {
                    out.push('@');
                    out.push_str(theme);
                    // Kept narrower only where the theme's name is broad.
                    if !theme.contains('.') && !tail.is_empty() {
                        out.push('.');
                        out.push_str(tail);
                    }
                    for _ in 0..len {
                        chars.next();
                    }
                    continue;
                }
            }
            _ => {}
        }
        out.push(ch);
    }
    out
}

/// Teach the highlighter every grammar here. Safe to call more than once.
pub fn register() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let registry = LanguageRegistry::singleton();
        for grammar in GRAMMARS {
            let highlights = modernise(grammar.highlights);
            registry.register(
                grammar.name,
                &GrammarConfig::new(
                    grammar.name,
                    (grammar.language)(),
                    vec![],
                    &highlights,
                    "",
                    "",
                ),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_query_compiles_against_its_grammar() {
        for grammar in GRAMMARS {
            let language = (grammar.language)();
            let query = modernise(grammar.highlights);
            if let Err(error) = tree_sitter::Query::new(&language, &query) {
                panic!("{}: {error}", grammar.name);
            }
        }
    }

    #[test]
    fn old_capture_names_become_the_theme_s() {
        assert_eq!(
            modernise(r#"(if) @conditional (x) @repeat.loop "@include" @keyword"#),
            r#"(if) @keyword (x) @keyword.loop "@include" @keyword"#
        );
        assert_eq!(
            modernise("(e) @escape ; @include here"),
            "(e) @string.escape ; @include here"
        );
        assert_eq!(modernise(r#""\"@x" @field"#), r#""\"@x" @property"#);
    }

    #[test]
    fn each_grammar_highlights_something() {
        use tree_sitter::StreamingIterator as _;
        let samples: &[(&str, &str)] = &[
            ("dockerfile", "FROM rust:1\nRUN cargo build\n"),
            ("nix", "{ pkgs }: pkgs.hello # hi\n"),
            ("haskell", "main :: IO ()\nmain = putStrLn \"hi\"\n"),
            ("vue", "<template><div>{{ a }}</div></template>\n"),
            ("xml", "<a href=\"x\">t</a>\n"),
            ("objc", "@interface A : NSObject\n@end\n"),
            ("julia", "function f(x)\n  return x + 1\nend\n"),
            ("ada", "procedure Main is\nbegin\n  null;\nend Main;\n"),
        ];
        for (name, source) in samples {
            let grammar = GRAMMARS.iter().find(|g| g.name == *name).unwrap();
            let language = (grammar.language)();
            let mut parser = tree_sitter::Parser::new();
            parser.set_language(&language).unwrap();
            let tree = parser.parse(source, None).unwrap();
            let query = tree_sitter::Query::new(&language, &modernise(grammar.highlights)).unwrap();
            let mut cursor = tree_sitter::QueryCursor::new();
            let mut captures = cursor.captures(&query, tree.root_node(), source.as_bytes());
            assert!(captures.next().is_some(), "{name} highlights nothing");
        }
    }
}
