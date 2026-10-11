//! GDScript, Godot's scripting language. GPUI Kit has no feature for it, so
//! its grammar is a dependency of Jig's own and registered with the
//! highlighter at startup, with highlights written for it here.

use gpui_kit::component::highlighter::{GrammarConfig, LanguageRegistry};

const HIGHLIGHTS: &str = include_str!("gdscript.scm");

/// Teach the highlighter GDScript. Safe to call more than once.
pub fn register() {
    LanguageRegistry::singleton().register("gdscript", &config());
}

fn config() -> GrammarConfig {
    GrammarConfig::new(
        "gdscript",
        tree_sitter_gdscript::LANGUAGE.into(),
        vec![],
        HIGHLIGHTS,
        "",
        "",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_compile_against_the_grammar() {
        let language: tree_sitter::Language = tree_sitter_gdscript::LANGUAGE.into();
        if let Err(error) = tree_sitter::Query::new(&language, HIGHLIGHTS) {
            panic!("gdscript.scm: {error}");
        }
    }

    /// Each capture's name and text, first pattern winning per node.
    fn captures(source: &str) -> Vec<(String, String)> {
        use tree_sitter::StreamingIterator as _;
        let language: tree_sitter::Language = tree_sitter_gdscript::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language).unwrap();
        let tree = parser.parse(source, None).unwrap();
        assert!(!tree.root_node().has_error(), "{}", tree.root_node());
        let query = tree_sitter::Query::new(&language, HIGHLIGHTS).unwrap();
        let mut cursor = tree_sitter::QueryCursor::new();
        let mut matches = cursor.captures(&query, tree.root_node(), source.as_bytes());
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        while let Some((m, ix)) = matches.next() {
            let capture = m.captures[*ix];
            if seen.insert(capture.node.byte_range()) {
                let name = query.capture_names()[capture.index as usize];
                let text = &source[capture.node.byte_range()];
                out.push((name.to_string(), text.to_string()));
            }
        }
        out
    }

    #[test]
    fn highlights_a_script() {
        let source = "\
extends CharacterBody3D
class_name Player

signal died(cause: String)
const SPEED := 5.0
@export var health: int = 10

func _physics_process(delta: float) -> void:
\tif health <= 0:
\t\tdied.emit(\"fell\")
\tvelocity.x = SPEED * delta # move
\t$Sprite.play()
";
        let got = captures(source);
        let has = |name: &str, text: &str| got.iter().any(|(n, t)| n == name && t == text);
        assert!(has("keyword", "extends"), "{got:?}");
        assert!(has("type", "CharacterBody3D"), "{got:?}");
        assert!(has("type", "Player"), "{got:?}");
        assert!(has("function", "died"), "{got:?}");
        assert!(has("constant", "SPEED"), "{got:?}");
        assert!(has("attribute", "export"), "{got:?}");
        assert!(has("keyword", "func"), "{got:?}");
        assert!(has("function", "_physics_process"), "{got:?}");
        assert!(has("variable.special", "delta"), "{got:?}");
        assert!(has("type", "float"), "{got:?}");
        assert!(has("function", "emit"), "{got:?}");
        assert!(has("string", "\"fell\""), "{got:?}");
        assert!(has("comment", "# move"), "{got:?}");
        assert!(has("number", "10"), "{got:?}");
    }
}
