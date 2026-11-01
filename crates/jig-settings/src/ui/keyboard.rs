//! The keyboard shortcuts Settings lists and lets you change, as data: what
//! each is called, what keys it has and what it runs. The app lists them
//! (`crate::shortcuts`) and binds them; the Settings window shows them.

use std::collections::BTreeMap;

use gpui_kit::component::kbd::Kbd;
use gpui_kit::{Action, Keystroke};

use crate::domain::keyboard::{self, Group};

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
        keyboard::keys_in(self.id, self.keys, changed)
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
    let others = all
        .iter()
        .filter(|other| other.id != shortcut.id)
        .map(|other| (other.label, other.keys_in(changed)));
    keyboard::sharing(&shortcut.keys_in(changed), others, normal)
}
