//! Each language's logo, for files in the tree and in tabs.

use std::path::Path;

use gpui_kit::Hsla;

use crate::languages;
use crate::settings::LanguageSettings;
use crate::theme;

const RUST: &[u8] = include_bytes!("../../../assets/icons/rust.svg");
const TYPESCRIPT: &[u8] = include_bytes!("../../../assets/icons/typescript.svg");
const JAVASCRIPT: &[u8] = include_bytes!("../../../assets/icons/javascript.svg");
const JAVA: &[u8] = include_bytes!("../../../assets/icons/java.svg");
const PYTHON: &[u8] = include_bytes!("../../../assets/icons/python.svg");
const GO: &[u8] = include_bytes!("../../../assets/icons/go.svg");
const TOML: &[u8] = include_bytes!("../../../assets/icons/toml.svg");
const JSON: &[u8] = include_bytes!("../../../assets/icons/json.svg");
const MARKDOWN: &[u8] = include_bytes!("../../../assets/icons/markdown.svg");

/// A language's logo and its color, in Ember's light and dark variants.
struct Logo {
    svg: &'static [u8],
    light: &'static str,
    dark: &'static str,
}

fn logo(language: &str) -> Option<Logo> {
    let (svg, light, dark) = match language {
        "rust" => (RUST, "#C85A16", "#F59A5B"),
        "typescript" | "tsx" => (TYPESCRIPT, "#2F6FCF", "#7AAEF0"),
        "javascript" => (JAVASCRIPT, "#A86B00", "#F2C46D"),
        "java" => (JAVA, "#C8443C", "#F07870"),
        "python" => (PYTHON, "#3D7BB8", "#6FA8DC"),
        "go" => (GO, "#148A7E", "#5CC7B8"),
        "toml" => (TOML, "#8A4FC2", "#C792EA"),
        "json" => (JSON, "#A86B00", "#F2C46D"),
        "markdown" => (MARKDOWN, "#7F776F", "#A39A90"),
        _ => return None,
    };
    Some(Logo { svg, light, dark })
}

/// The logo for `path`'s language and the color to draw it in, or `None`
/// for a file in no bundled language.
pub fn for_path(
    path: &Path,
    settings: &LanguageSettings,
    dark: bool,
) -> Option<(&'static [u8], Hsla)> {
    let logo = logo(languages::written_in(path, settings)?.name)?;
    let color = theme::parse_hex(if dark { logo.dark } else { logo.light })?;
    Some((logo.svg, color))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_language_has_a_logo() {
        for language in languages::BUNDLED {
            let logo =
                logo(language.name).unwrap_or_else(|| panic!("{} has no logo", language.name));
            assert!(std::str::from_utf8(logo.svg).unwrap().starts_with("<svg"));
            assert!(
                theme::parse_hex(logo.light).is_some() && theme::parse_hex(logo.dark).is_some()
            );
        }
    }

    #[test]
    fn files_get_their_language_s_logo() {
        let settings = LanguageSettings::default();
        let svg = |path: &str| for_path(Path::new(path), &settings, true).map(|(svg, _)| svg);
        assert_eq!(svg("src/main.rs"), Some(RUST));
        assert_eq!(svg("App.tsx"), Some(TYPESCRIPT));
        assert_eq!(svg("Cargo.toml"), Some(TOML));
        assert_eq!(svg("Makefile"), None);
        assert_eq!(svg("notes.txt"), None);
    }
}
