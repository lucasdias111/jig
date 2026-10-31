//! The keyboard shortcuts Settings lists and lets you change, as data: what
//! each is called and what keys it has, before and after `[shortcuts]` in
//! `settings.toml`. The app lists them (`crate::shortcuts`) and binds them;
//! the Settings window shows them.

use std::collections::BTreeMap;

use gpui_kit::component::kbd::Kbd;
use gpui_kit::{Action, Keystroke};

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

pub struct Shortcut {
    /// Its name in `settings.toml`.
    pub id: &'static str,
    pub label: &'static str,
    pub group: Group,
    /// The default keystrokes; the first is the one shown.
    pub keys: &'static [&'static str],
    /// Where it's bound: `None` for anywhere in the window.
    pub contexts: &'static [Option<&'static str>],
    pub action: fn() -> Box<dyn Action>,
}

impl Shortcut {
    pub fn action(&self) -> Box<dyn Action> {
        (self.action)()
    }

    /// The keystrokes it's bound to, with `changed` (from `settings.toml`)
    /// applied: one, or none.
    pub fn keys_in(&self, changed: &BTreeMap<String, String>) -> Vec<String> {
        match changed.get(self.id) {
            Some(keys) if keys.trim().is_empty() => Vec::new(),
            Some(keys) => vec![keys.trim().to_string()],
            None => self.keys.iter().map(|keys| keys.to_string()).collect(),
        }
    }
}

/// How `keys` look on this platform: `⇧⌘D` and `⌘⏎` on a Mac,
/// `Ctrl+Shift+D` elsewhere. Text that isn't a keystroke is shown as it is.
pub fn display(keys: &str) -> String {
    keys.split_whitespace()
        .map(|keys| {
            Keystroke::parse(keys).map_or_else(|_| keys.to_string(), |keys| Kbd::format(&keys))
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The other shortcuts sharing a keystroke with `shortcut`, by label.
pub fn clashes(
    shortcut: &Shortcut,
    all: &[Shortcut],
    changed: &BTreeMap<String, String>,
) -> Vec<&'static str> {
    let normal = |keys: &str| Keystroke::parse(keys).map(|keys| keys.unparse()).ok();
    let mine: Vec<_> = shortcut
        .keys_in(changed)
        .iter()
        .filter_map(|keys| normal(keys))
        .collect();
    all.iter()
        .filter(|other| other.id != shortcut.id)
        .filter(|other| {
            other
                .keys_in(changed)
                .iter()
                .filter_map(|keys| normal(keys))
                .any(|keys| mine.contains(&keys))
        })
        .map(|other| other.label)
        .collect()
}
