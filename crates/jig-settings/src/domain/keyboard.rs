//! The rules of the keyboard shortcuts Settings lists and lets you change:
//! which keys one has once `[shortcuts]` in `settings.toml` has its say,
//! and which share a keystroke. The shortcuts themselves, with the actions
//! they run, are `ui::keyboard`'s.

use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    File,
    Editing,
    Navigation,
    Agent,
    View,
    Run,
}

impl Group {
    pub const ALL: [Group; 6] = [
        Group::Editing,
        Group::Navigation,
        Group::File,
        Group::Agent,
        Group::View,
        Group::Run,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Group::File => "Files and tabs",
            Group::Editing => "Editing",
            Group::Navigation => "Navigation",
            Group::Agent => "Agent",
            Group::View => "View",
            Group::Run => "Run and debug",
        }
    }
}

/// The keystrokes a shortcut called `id` with `defaults` is bound to, with
/// `changed` (from `settings.toml`) applied: one, or none.
pub fn keys_in(id: &str, defaults: &[&str], changed: &BTreeMap<String, String>) -> Vec<String> {
    match changed.get(id) {
        Some(keys) if keys.trim().is_empty() => Vec::new(),
        Some(keys) => vec![keys.trim().to_string()],
        None => defaults.iter().map(|keys| keys.to_string()).collect(),
    }
}

/// The labels of `others`, each a label and its keys, sharing a keystroke
/// with `mine`, the keystrokes compared as `normal` writes them (keys it
/// can't read count as none).
pub fn sharing<'a>(
    mine: &[String],
    others: impl IntoIterator<Item = (&'a str, Vec<String>)>,
    normal: impl Fn(&str) -> Option<String>,
) -> Vec<&'a str> {
    let mine: Vec<_> = mine.iter().filter_map(|keys| normal(keys)).collect();
    others
        .into_iter()
        .filter(|(_, keys)| {
            keys.iter()
                .filter_map(|keys| normal(keys))
                .any(|keys| mine.contains(&keys))
        })
        .map(|(label, _)| label)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_keys_replace_the_defaults() {
        let changed = BTreeMap::from([
            ("dup".to_string(), "cmd-d".to_string()),
            ("gone".to_string(), " ".to_string()),
        ]);
        assert_eq!(keys_in("dup", &["cmd-shift-d"], &changed), ["cmd-d"]);
        assert!(keys_in("gone", &["cmd-g"], &changed).is_empty());
        assert_eq!(keys_in("kept", &["a", "b"], &changed), ["a", "b"]);
    }

    #[test]
    fn shares_keystrokes_as_normalised() {
        let normal = |keys: &str| Some(keys.to_lowercase());
        let others = [
            ("Up", vec!["CMD-U".to_string()]),
            ("Down", vec!["cmd-j".into()]),
        ];
        assert_eq!(sharing(&["cmd-u".into()], others, normal), ["Up"]);
    }
}
