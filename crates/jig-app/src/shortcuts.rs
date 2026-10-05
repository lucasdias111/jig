//! The keyboard shortcuts Settings lists and lets you change. Each has a
//! default; a change is kept in `settings.toml` under `[shortcuts]` as
//! GPUI writes keystrokes (`cmd-shift-d`), `""` for none, and the keymap
//! is rebuilt on the spot. Bindings that aren't listed here (the editor's
//! own, the file tree's, ⌘1…9) stay as they are.

use std::collections::BTreeMap;

use gpui_kit::component::input::{GoToDefinition, Replace};
use gpui_kit::*;

use crate::settings;
use crate::settings_window::OpenSettings;
use crate::workspace::*;

/// The code editor, and not the other fields that are inputs too.
pub const CODE: &str = "CodeEditor > Input";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    File,
    Editing,
    Navigation,
    Jigs,
    View,
    Run,
}

impl Group {
    pub const ALL: [Group; 6] = [
        Group::Editing,
        Group::Navigation,
        Group::File,
        Group::Jigs,
        Group::View,
        Group::Run,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Group::File => "Files and tabs",
            Group::Editing => "Editing",
            Group::Navigation => "Navigation",
            Group::Jigs => "Jigs",
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
    contexts: &'static [Option<&'static str>],
    action: fn() -> Box<dyn Action>,
}

fn boxed<A: Action + Default>() -> Box<dyn Action> {
    Box::new(A::default())
}

const ANYWHERE: &[Option<&str>] = &[None];
const IN_CODE: &[Option<&str>] = &[Some(CODE)];
const IN_INPUT: &[Option<&str>] = &[Some("Input")];

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

/// ⌥⌘R as in IntelliJ on the Mac; elsewhere it would be ⌃⌥R, which is Run….
const RESUME_KEYS: &[&str] = if cfg!(target_os = "macos") {
    &["f9", "alt-cmd-r"]
} else {
    &["f9"]
};

pub const ALL: &[Shortcut] = &[
    // Editing.
    shortcut!(
        "toggle_line_comment",
        "Toggle line comment",
        Editing,
        // ⌘/ as in VS Code too.
        ["secondary-'", "secondary-/"],
        IN_CODE,
        ToggleLineComment
    ),
    shortcut!(
        "move_line_up",
        "Move line up",
        Editing,
        ["alt-up"],
        IN_CODE,
        MoveLineUp
    ),
    shortcut!(
        "move_line_down",
        "Move line down",
        Editing,
        ["alt-down"],
        IN_CODE,
        MoveLineDown
    ),
    shortcut!(
        "duplicate_line",
        "Duplicate line",
        Editing,
        ["secondary-shift-d"],
        IN_CODE,
        DuplicateLine
    ),
    shortcut!(
        "delete_line",
        "Delete line",
        Editing,
        ["secondary-shift-backspace"],
        IN_CODE,
        DeleteLine
    ),
    shortcut!(
        "select_line",
        "Select line",
        Editing,
        ["secondary-l"],
        IN_CODE,
        SelectLine
    ),
    shortcut!(
        "insert_line_below",
        "New line below",
        Editing,
        ["secondary-enter"],
        IN_CODE,
        InsertLineBelow
    ),
    shortcut!(
        "insert_line_above",
        "New line above",
        Editing,
        ["secondary-shift-enter"],
        IN_CODE,
        InsertLineAbove
    ),
    shortcut!(
        "rename_symbol",
        "Rename symbol",
        Editing,
        ["f2"],
        IN_INPUT,
        RenameSymbol
    ),
    // ⇧⌘F is GPUI Kit's Replace in the editor; Replace moves to ⌘R, as
    // in IntelliJ. Elsewhere ⌃R is Run, so Replace takes the usual ⌃H.
    shortcut!(
        "replace",
        "Replace in file",
        Editing,
        [if cfg!(target_os = "macos") {
            "cmd-r"
        } else {
            "ctrl-h"
        }],
        IN_INPUT,
        Replace
    ),
    // Navigation.
    shortcut!(
        "go_to_file",
        "Go to file",
        Navigation,
        ["secondary-p"],
        ANYWHERE,
        GoToFile
    ),
    // As in VS Code.
    shortcut!(
        "go_to_line",
        "Go to line",
        Navigation,
        ["ctrl-g"],
        ANYWHERE,
        GoToLine
    ),
    shortcut!(
        "find_in_files",
        "Find in files",
        Navigation,
        ["secondary-shift-f"],
        &[None, Some("Input")],
        FindInFiles
    ),
    // ⌘-click does the same.
    shortcut!(
        "go_to_definition",
        "Go to definition",
        Navigation,
        ["f12"],
        IN_INPUT,
        GoToDefinition
    ),
    shortcut!(
        "find_references",
        "Find all references",
        Navigation,
        ["shift-f12"],
        IN_INPUT,
        FindReferences
    ),
    shortcut!(
        "next_tab",
        "Next tab",
        Navigation,
        ["secondary-shift-]", "ctrl-tab"],
        ANYWHERE,
        NextTab
    ),
    shortcut!(
        "previous_tab",
        "Previous tab",
        Navigation,
        ["secondary-shift-[", "ctrl-shift-tab"],
        ANYWHERE,
        PreviousTab
    ),
    // Files and tabs.
    shortcut!(
        "new_file",
        "New file",
        File,
        ["secondary-n"],
        ANYWHERE,
        NewFile
    ),
    shortcut!("open", "Open", File, ["secondary-o"], ANYWHERE, Open),
    shortcut!("save", "Save", File, ["secondary-s"], ANYWHERE, Save),
    shortcut!(
        "save_as",
        "Save as",
        File,
        ["secondary-shift-s"],
        ANYWHERE,
        SaveAs
    ),
    shortcut!(
        "close_tab",
        "Close tab",
        File,
        ["secondary-w"],
        ANYWHERE,
        CloseTab
    ),
    shortcut!(
        "close_window",
        "Close window",
        File,
        ["secondary-shift-w"],
        ANYWHERE,
        CloseWindow
    ),
    shortcut!(
        "settings",
        "Settings",
        File,
        ["secondary-,"],
        ANYWHERE,
        OpenSettings
    ),
    shortcut!("quit", "Quit", File, ["secondary-q"], ANYWHERE, Quit),
    // Jigs.
    shortcut!(
        "run_jig",
        "Run jig",
        Jigs,
        ["secondary-k"],
        ANYWHERE,
        OpenCommand
    ),
    shortcut!(
        "add_jig",
        "Add jig",
        Jigs,
        ["secondary-shift-k"],
        ANYWHERE,
        AddCommand
    ),
    // View.
    shortcut!(
        "toggle_sidebar",
        "Toggle sidebar",
        View,
        ["secondary-b"],
        ANYWHERE,
        ToggleSidebar
    ),
    shortcut!(
        "show_files",
        "Show files",
        View,
        ["secondary-shift-e"],
        ANYWHERE,
        FocusFileTree
    ),
    shortcut!(
        "toggle_run_panel",
        "Toggle run panel",
        View,
        ["secondary-j"],
        ANYWHERE,
        ToggleRunPanel
    ),
    // As in VS Code.
    shortcut!(
        "toggle_git_panel",
        "Show changes",
        View,
        ["ctrl-shift-g"],
        ANYWHERE,
        ToggleGitPanel
    ),
    shortcut!(
        "zoom_in",
        "Bigger code font",
        View,
        ["secondary-="],
        ANYWHERE,
        ZoomIn
    ),
    shortcut!(
        "zoom_out",
        "Smaller code font",
        View,
        ["secondary--"],
        ANYWHERE,
        ZoomOut
    ),
    shortcut!(
        "reset_zoom",
        "Default code font size",
        View,
        ["secondary-0"],
        ANYWHERE,
        ResetZoom
    ),
    // Run and debug, as in IntelliJ on the Mac.
    shortcut!("run", "Run", Run, ["ctrl-r"], ANYWHERE, RunSelected),
    shortcut!(
        "choose_run",
        "Run…",
        Run,
        ["ctrl-alt-r"],
        ANYWHERE,
        ChooseRunConfiguration
    ),
    shortcut!("debug", "Debug", Run, ["ctrl-d"], ANYWHERE, DebugSelected),
    shortcut!("stop", "Stop", Run, ["secondary-f2"], ANYWHERE, StopRun),
    shortcut!(
        "toggle_breakpoint",
        "Toggle breakpoint",
        Run,
        ["secondary-f8"],
        ANYWHERE,
        ToggleBreakpoint
    ),
    shortcut!("resume", "Resume", Run, RESUME_KEYS, ANYWHERE, Resume),
    shortcut!("step_over", "Step over", Run, ["f8"], ANYWHERE, StepOver),
    shortcut!("step_into", "Step into", Run, ["f7"], ANYWHERE, StepInto),
    shortcut!("step_out", "Step out", Run, ["shift-f8"], ANYWHERE, StepOut),
];

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

    fn bindings(&self, changed: &BTreeMap<String, String>) -> Vec<KeyBinding> {
        let mut bindings = Vec::new();
        for keys in self.keys_in(changed) {
            for context in self.contexts {
                let context = context.map(|context| {
                    KeyBindingContextPredicate::parse(context)
                        .expect("contexts are valid")
                        .into()
                });
                match KeyBinding::load(
                    &keys,
                    self.action(),
                    context,
                    false,
                    None,
                    &DummyKeyboardMapper,
                ) {
                    Ok(binding) => bindings.push(binding),
                    Err(error) => eprintln!("jig: shortcut {} ({keys}): {error}", self.id),
                }
            }
        }
        bindings
    }
}

/// Every listed shortcut, bound with `changed` applied.
pub fn bindings(changed: &BTreeMap<String, String>) -> Vec<KeyBinding> {
    ALL.iter()
        .flat_map(|shortcut| shortcut.bindings(changed))
        .collect()
}

/// How `keys` look on this platform: `⇧⌘D` on a Mac, `ctrl-shift-d`
/// elsewhere. Text that isn't a keystroke is shown as it is.
pub fn display(keys: &str) -> String {
    keys.split_whitespace()
        .map(|keys| {
            Keystroke::parse(keys).map_or_else(|_| keys.to_string(), |keys| keys.to_string())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The other shortcuts sharing a keystroke with `shortcut`, by label.
pub fn clashes(shortcut: &Shortcut, changed: &BTreeMap<String, String>) -> Vec<&'static str> {
    let normal = |keys: &str| Keystroke::parse(keys).map(|keys| keys.unparse()).ok();
    let mine: Vec<_> = shortcut
        .keys_in(changed)
        .iter()
        .filter_map(|keys| normal(keys))
        .collect();
    ALL.iter()
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
    let actions: Vec<_> = ALL.iter().map(Shortcut::action).collect();
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
mod tests {
    use std::collections::{BTreeMap, HashSet};

    use super::{ALL, bindings, clashes};

    #[test]
    fn every_default_parses_and_ids_are_unique() {
        let ids: HashSet<_> = ALL.iter().map(|shortcut| shortcut.id).collect();
        assert_eq!(ids.len(), ALL.len());
        let expected: usize = ALL
            .iter()
            .map(|shortcut| shortcut.keys.len() * shortcut.contexts.len())
            .sum();
        assert_eq!(bindings(&BTreeMap::new()).len(), expected);
    }

    #[test]
    fn defaults_dont_clash() {
        for shortcut in ALL {
            assert_eq!(
                clashes(shortcut, &BTreeMap::new()),
                Vec::<&str>::new(),
                "{}",
                shortcut.id
            );
        }
    }

    #[test]
    fn a_change_replaces_the_defaults_and_empty_unbinds() {
        let save = ALL.iter().find(|shortcut| shortcut.id == "save").unwrap();
        let mut changed = BTreeMap::new();
        changed.insert("save".to_string(), "secondary-shift-d".to_string());
        assert_eq!(save.keys_in(&changed), ["secondary-shift-d"]);
        assert_eq!(clashes(save, &changed), ["Duplicate line"]);
        changed.insert("save".to_string(), String::new());
        assert!(save.keys_in(&changed).is_empty());
    }
}
