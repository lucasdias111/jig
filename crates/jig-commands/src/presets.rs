//! Preset commands: loading them from TOML, filtering them as the user types,
//! and working out which part of the buffer a command targets.

use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::Deserialize;

const DEFAULT_COMMANDS: &str = include_str!("../../../assets/default-commands.toml");

/// Which part of the buffer a command reads and replaces.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scope {
    /// The selection, or the current line when nothing is selected.
    #[default]
    Selection,
    /// An empty range at the cursor: the command inserts code.
    Cursor,
    /// The whole file.
    File,
}

impl Scope {
    /// The byte range this scope targets in `text`.
    pub fn target(self, text: &str, selection: Range<usize>, cursor: usize) -> Range<usize> {
        match self {
            Scope::Selection if selection.is_empty() => line_at(text, selection.start),
            Scope::Selection => selection,
            Scope::Cursor => cursor..cursor,
            Scope::File => 0..text.len(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(default)]
    pub scope: Scope,
    pub prompt: String,
}

/// What the user chose to run from the command input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    /// The preset's name, or `None` for a custom instruction.
    pub name: Option<String>,
    pub instruction: String,
    pub scope: Scope,
}

impl Invocation {
    pub fn preset(preset: &Preset) -> Self {
        Self {
            name: Some(preset.name.clone()),
            instruction: preset.prompt.clone(),
            scope: preset.scope,
        }
    }

    /// A typed instruction. It works on the selection when there is one and
    /// inserts at the cursor otherwise.
    pub fn custom(text: &str, has_selection: bool) -> Self {
        let scope = if has_selection {
            Scope::Selection
        } else {
            Scope::Cursor
        };
        Self {
            name: None,
            instruction: text.trim().to_string(),
            scope,
        }
    }
}

#[derive(Deserialize)]
struct PresetFile {
    #[serde(default, rename = "command")]
    commands: Vec<Preset>,
}

pub fn parse(source: &str) -> Result<Vec<Preset>> {
    Ok(toml::from_str::<PresetFile>(source)?.commands)
}

/// The built-in presets.
pub fn defaults() -> Vec<Preset> {
    parse(DEFAULT_COMMANDS).expect("bundled default-commands.toml is valid")
}

/// `~/.config/jig/commands.toml`, honouring `XDG_CONFIG_HOME`.
pub fn user_commands_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    Some(config.join("jig").join("commands.toml"))
}

/// The built-in presets merged with the user's file, if it exists. A user
/// preset replaces a built-in one with the same name; new ones go last.
pub fn load(user_path: Option<&Path>) -> Result<Vec<Preset>> {
    let mut presets = defaults();
    let Some(path) = user_path.filter(|path| path.exists()) else {
        return Ok(presets);
    };
    let source =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let user = parse(&source).with_context(|| format!("parsing {}", path.display()))?;
    for preset in user {
        match presets
            .iter_mut()
            .find(|existing| existing.name.eq_ignore_ascii_case(&preset.name))
        {
            Some(existing) => *existing = preset,
            None => presets.push(preset),
        }
    }
    Ok(presets)
}

/// Indices of the presets whose names match `query`, best match first. An
/// empty query matches everything in file order.
pub fn filter(presets: &[Preset], query: &str) -> Vec<usize> {
    let query = query.trim().to_lowercase();
    let mut matches: Vec<(u32, usize)> = presets
        .iter()
        .enumerate()
        .filter_map(|(index, preset)| {
            score(&preset.name.to_lowercase(), &query).map(|score| (score, index))
        })
        .collect();
    matches.sort();
    matches.into_iter().map(|(_, index)| index).collect()
}

/// Lower is better: prefix, then substring, then a subsequence ranked by how
/// spread out its characters are. `None` when `query` isn't a subsequence.
fn score(name: &str, query: &str) -> Option<u32> {
    if name.starts_with(query) {
        return Some(0);
    }
    if name.contains(query) {
        return Some(1);
    }
    let mut gaps = 0;
    let mut chars = name.chars();
    for wanted in query.chars() {
        let mut skipped = 0;
        loop {
            match chars.next() {
                Some(c) if c == wanted => break,
                Some(_) => skipped += 1,
                None => return None,
            }
        }
        gaps += skipped;
    }
    Some(2 + gaps)
}

/// The line containing `offset`, without its trailing newline.
fn line_at(text: &str, offset: usize) -> Range<usize> {
    let offset = offset.min(text.len());
    let start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let end = text[offset..].find('\n').map_or(text.len(), |i| offset + i);
    start..end
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(presets: &[Preset], indices: &[usize]) -> Vec<String> {
        indices.iter().map(|&i| presets[i].name.clone()).collect()
    }

    #[test]
    fn defaults_parse() {
        let presets = defaults();
        assert!(
            presets
                .iter()
                .any(|p| p.name == "Add docs" && p.scope == Scope::Selection)
        );
        assert!(presets.iter().any(|p| p.scope == Scope::Cursor));
    }

    #[test]
    fn scope_defaults_to_selection() {
        let presets = parse("[[command]]\nname = \"X\"\nprompt = \"do x\"\n").unwrap();
        assert_eq!(presets[0].scope, Scope::Selection);
    }

    #[test]
    fn bad_scope_is_an_error() {
        assert!(parse("[[command]]\nname = \"X\"\nscope = \"repo\"\nprompt = \"p\"\n").is_err());
    }

    #[test]
    fn user_presets_override_and_extend() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("commands.toml");
        std::fs::write(
            &path,
            "[[command]]\nname = \"add docs\"\nprompt = \"mine\"\n\n[[command]]\nname = \"Create controller\"\nscope = \"cursor\"\nprompt = \"p\"\n",
        )
        .unwrap();
        let presets = load(Some(&path)).unwrap();
        assert_eq!(
            presets
                .iter()
                .find(|p| p.name == "add docs")
                .unwrap()
                .prompt,
            "mine"
        );
        assert!(!presets.iter().any(|p| p.name == "Add docs"));
        assert_eq!(presets.last().unwrap().name, "Create controller");
        assert_eq!(presets.len(), defaults().len() + 1);
    }

    #[test]
    fn missing_user_file_gives_defaults() {
        assert_eq!(
            load(Some(Path::new("/nonexistent/commands.toml"))).unwrap(),
            defaults()
        );
        assert_eq!(load(None).unwrap(), defaults());
    }

    #[test]
    fn filter_ranks_prefix_then_substring_then_subsequence() {
        let presets = parse(
            r#"
            [[command]]
            name = "Extract function"
            prompt = "p"
            [[command]]
            name = "Add docs"
            prompt = "p"
            [[command]]
            name = "Fix"
            prompt = "p"
            [[command]]
            name = "Add tests"
            prompt = "p"
            "#,
        )
        .unwrap();
        assert_eq!(filter(&presets, "").len(), 4);
        assert_eq!(
            names(&presets, &filter(&presets, "add")),
            ["Add docs", "Add tests"]
        );
        assert_eq!(names(&presets, &filter(&presets, "docs")), ["Add docs"]);
        assert_eq!(
            names(&presets, &filter(&presets, "ADS")),
            ["Add tests", "Add docs"]
        );
        assert_eq!(
            names(&presets, &filter(&presets, "fn")),
            ["Extract function"]
        );
        assert!(filter(&presets, "zzz").is_empty());
    }

    #[test]
    fn scope_targets() {
        let text = "fn a() {\n    one();\n}\n";
        let line_2 = text.find("one").unwrap();
        assert_eq!(
            &text[Scope::Selection.target(text, line_2..line_2, line_2)],
            "    one();"
        );
        assert_eq!(Scope::Selection.target(text, 0..2, 2), 0..2);
        assert_eq!(Scope::Cursor.target(text, 0..2, 2), 2..2);
        assert_eq!(Scope::File.target(text, 0..2, 2), 0..text.len());
        // Cursor at the very end, after the final newline: an empty last line.
        assert_eq!(
            Scope::Selection.target(text, text.len()..text.len(), text.len()),
            text.len()..text.len()
        );
    }

    #[test]
    fn custom_invocation_scope() {
        assert_eq!(
            Invocation::custom(" make it async ", true).scope,
            Scope::Selection
        );
        assert_eq!(
            Invocation::custom("insert a struct", false).scope,
            Scope::Cursor
        );
        assert_eq!(
            Invocation::custom(" make it async ", true).instruction,
            "make it async"
        );
    }
}
