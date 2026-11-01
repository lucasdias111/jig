//! The keyboard shortcuts Settings lists and lets you change. Each has a
//! default; a change is kept in `settings.toml` under `[shortcuts]` as
//! GPUI writes keystrokes (`cmd-shift-d`), `""` for none, and the keymap
//! is rebuilt on the spot. Bindings that aren't listed here (the editor's
//! own, the file tree's) stay as they are.

use std::collections::BTreeMap;

use gpui_kit::component::input::{GoToDefinition, Replace, Search, ToggleCodeActions};
use gpui_kit::*;

use crate::workspace::*;
use jig_preferences::settings_window::OpenSettings;
use jig_settings::{self as settings, Shortcut, ShortcutGroup as Group};

/// The code editor, and not the other fields that are inputs too.
pub const CODE: &str = "CodeEditor > Input";

fn boxed<A: Action + Default>() -> Box<dyn Action> {
    Box::new(A::default())
}

const ANYWHERE: &[Option<&str>] = &[None];
const IN_CODE: &[Option<&str>] = &[Some(CODE)];
const IN_INPUT: &[Option<&str>] = &[Some("Input")];
/// The workspace, and not the other windows' fields.
const IN_WORKSPACE: &[Option<&str>] = &[Some(crate::workspace::CONTEXT)];

/// ⌃1…9 on a Mac, where ⌘1…4 are the modes; Alt-1…9 elsewhere, where
/// Ctrl is the secondary key.
const TAB_KEYS: [&[&str]; 9] = if cfg!(target_os = "macos") {
    [
        &["ctrl-1"],
        &["ctrl-2"],
        &["ctrl-3"],
        &["ctrl-4"],
        &["ctrl-5"],
        &["ctrl-6"],
        &["ctrl-7"],
        &["ctrl-8"],
        &["ctrl-9"],
    ]
} else {
    [
        &["alt-1"],
        &["alt-2"],
        &["alt-3"],
        &["alt-4"],
        &["alt-5"],
        &["alt-6"],
        &["alt-7"],
        &["alt-8"],
        &["alt-9"],
    ]
};

fn tab<const N: usize>() -> Box<dyn Action> {
    Box::new(ActivateTab(N))
}

/// The shortcut to show tab `N + 1`.
const fn tab_shortcut<const N: usize>(id: &'static str, label: &'static str) -> Shortcut {
    Shortcut {
        id,
        label,
        group: Group::File,
        keys: TAB_KEYS[N],
        contexts: IN_WORKSPACE,
        action: tab::<N>,
    }
}

macro_rules! shortcut {
    ($id:literal, $label:literal, $group:ident, [$($keys:expr),+], $contexts:expr, $action:ty) => {
        Shortcut {
            id: $id,
            label: $label,
            group: Group::$group,
            keys: &[$($keys),+],
            contexts: $contexts,
            action: boxed::<$action>,
        }
    };
    ($id:literal, $label:literal, $group:ident, $keys:expr, $contexts:expr, $action:ty) => {
        Shortcut {
            id: $id,
            label: $label,
            group: Group::$group,
            keys: $keys,
            contexts: $contexts,
            action: boxed::<$action>,
        }
    };
}

mod table;
pub use table::ALL;

/// Every listed shortcut, bound with `changed` applied.
pub fn bindings(changed: &BTreeMap<String, String>) -> Vec<KeyBinding> {
    ALL.iter()
        .flat_map(|shortcut| bindings_of(shortcut, changed))
        .collect()
}

/// The key bindings `shortcut` makes, with `changed` (from `settings.toml`)
/// applied.
fn bindings_of(shortcut: &Shortcut, changed: &BTreeMap<String, String>) -> Vec<KeyBinding> {
    let mut bindings = Vec::new();
    for keys in shortcut.keys_in(changed) {
        for context in shortcut.contexts {
            let context = context.map(|context| {
                KeyBindingContextPredicate::parse(context)
                    .expect("contexts are valid")
                    .into()
            });
            match KeyBinding::load(
                &keys,
                shortcut.action(),
                context,
                false,
                None,
                &DummyKeyboardMapper,
            ) {
                Ok(binding) => bindings.push(binding),
                Err(error) => eprintln!("jig: shortcut {} ({keys}): {error}", shortcut.id),
            }
        }
    }
    bindings
}

/// The keymap without the listed shortcuts, and the changes last bound.
struct Keymap {
    base: Vec<KeyBinding>,
    applied: BTreeMap<String, String>,
}

impl Global for Keymap {}

/// Take over the listed shortcuts from whatever bound them (GPUI Kit binds
/// some of the same actions) and rebind them whenever Settings changes
/// one. Runs after everything else has bound its keys.
pub fn init(cx: &mut App) {
    // GPUI Kit's search panel is replaced by Jig's, so its keys go too.
    let taken_over: [Box<dyn Action>; 2] = [Box::new(Search), Box::new(Replace)];
    let actions: Vec<_> = ALL.iter().map(Shortcut::action).chain(taken_over).collect();
    let base = cx
        .key_bindings()
        .borrow()
        .bindings()
        .filter(|binding| {
            !actions
                .iter()
                .any(|action| action.partial_eq(binding.action()))
        })
        .cloned()
        .collect();
    let applied = settings::get(cx).shortcuts;
    cx.set_global(Keymap {
        base,
        applied: applied.clone(),
    });
    bind(&applied, cx);
    cx.observe_global::<settings::AppSettings>(|cx| {
        let changed = settings::get(cx).shortcuts;
        if cx.global::<Keymap>().applied != changed {
            cx.global_mut::<Keymap>().applied = changed.clone();
            bind(&changed, cx);
        }
    })
    .detach();
}

fn bind(changed: &BTreeMap<String, String>, cx: &mut App) {
    let base = cx.global::<Keymap>().base.clone();
    cx.clear_key_bindings();
    cx.bind_keys(base);
    cx.bind_keys(bindings(changed));
    // The menus show the shortcuts bound when they're set.
    crate::menus::set(cx);
}

#[cfg(test)]
mod tests;
