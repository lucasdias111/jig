//! The macOS menu bar: Jig, File, Edit, View, Run and Git.

use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::*;

use crate::settings_window::OpenSettings;
use crate::workspace::{
    AddCommand, ChooseRunConfiguration, CloseTab, CloseWindow, DebugSelected, DeleteLine,
    DuplicateLine, EditAgentsFile, EditCommands, EditDebuggers, EditRunConfigurations, Find,
    FindAndReplace, FindInFiles, FindReferences, FocusFileTree, GoToFile, GoToLine, MoveLineDown,
    MoveLineUp, NewFile, NextProblem, NextTab, Open, OpenCommand, PreviousProblem, PreviousTab,
    Quit, RenameSymbol, ResetZoom, Resume, RunSelected, Save, SaveAs, SelectLine, StepInto,
    StepOut, StepOver, StopRun, SwitchBranch, ToggleBreakpoint, ToggleGitPanel, ToggleLineComment,
    ToggleRunPanel, ToggleSidebar, ZoomIn, ZoomOut,
};

actions!(jig, [About, Hide, HideOthers, ShowAll]);

pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
    ]);
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &About, cx| {
        let Some(window) = cx.active_window() else {
            return;
        };
        window
            .update(cx, |_, window, cx| {
                let detail = format!(
                    "Version {}\nAI jigs at the cursor.",
                    env!("CARGO_PKG_VERSION")
                );
                // Nothing to do with the answer.
                drop(window.prompt(PromptLevel::Info, "Jig", Some(&detail), &["OK"], cx));
            })
            .ok();
    });
    set(cx);
}

/// Set the menus, showing the shortcuts bound now. Again whenever those
/// change.
pub fn set(cx: &mut App) {
    cx.set_menus([
        // macOS titles this menu with the app's name.
        menu(
            "Jig",
            vec![
                MenuItem::action("About Jig", About),
                MenuItem::separator(),
                MenuItem::action("Settings…", OpenSettings),
                MenuItem::separator(),
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Jig", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Jig", Quit),
            ],
        ),
        menu(
            "File",
            vec![
                MenuItem::action("New File", NewFile),
                MenuItem::action("Open…", Open),
                MenuItem::action("Go to File…", GoToFile),
                MenuItem::action("Go to Line…", GoToLine),
                MenuItem::separator(),
                MenuItem::action("Save", Save),
                MenuItem::action("Save As…", SaveAs),
                MenuItem::separator(),
                MenuItem::action("Close Tab", CloseTab),
                MenuItem::action("Close Window", CloseWindow),
            ],
        ),
        menu(
            "Edit",
            vec![
                MenuItem::os_action("Undo", Undo, OsAction::Undo),
                MenuItem::os_action("Redo", Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Cut", Cut, OsAction::Cut),
                MenuItem::os_action("Copy", Copy, OsAction::Copy),
                MenuItem::os_action("Paste", Paste, OsAction::Paste),
                MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
                MenuItem::separator(),
                MenuItem::action("Find…", Find),
                MenuItem::action("Find and Replace…", FindAndReplace),
                MenuItem::action("Find in Files…", FindInFiles),
                MenuItem::action("Find All References", FindReferences),
                MenuItem::action("Next Problem", NextProblem),
                MenuItem::action("Previous Problem", PreviousProblem),
                MenuItem::action("Rename Symbol…", RenameSymbol),
                MenuItem::action("Toggle Line Comment", ToggleLineComment),
                MenuItem::separator(),
                MenuItem::action("Move Line Up", MoveLineUp),
                MenuItem::action("Move Line Down", MoveLineDown),
                MenuItem::action("Duplicate Line", DuplicateLine),
                MenuItem::action("Delete Line", DeleteLine),
                MenuItem::action("Select Line", SelectLine),
                MenuItem::separator(),
                MenuItem::action("Run Jig…", OpenCommand),
                MenuItem::action("Add Jig…", AddCommand),
                MenuItem::action("Edit Jigs File", EditCommands),
                MenuItem::action("Edit AGENTS.md", EditAgentsFile),
            ],
        ),
        menu(
            "View",
            vec![
                MenuItem::action("Toggle Sidebar", ToggleSidebar),
                MenuItem::action("Show Files", FocusFileTree),
                MenuItem::separator(),
                MenuItem::action("Show Next Tab", NextTab),
                MenuItem::action("Show Previous Tab", PreviousTab),
                MenuItem::separator(),
                MenuItem::action("Toggle Run Panel", ToggleRunPanel),
                MenuItem::separator(),
                MenuItem::action("Bigger Code Font", ZoomIn),
                MenuItem::action("Smaller Code Font", ZoomOut),
                MenuItem::action("Default Code Font Size", ResetZoom),
            ],
        ),
        menu(
            "Run",
            vec![
                MenuItem::action("Run", RunSelected),
                MenuItem::action("Run…", ChooseRunConfiguration),
                MenuItem::action("Debug", DebugSelected),
                MenuItem::action("Stop", StopRun),
                MenuItem::separator(),
                MenuItem::action("Toggle Breakpoint", ToggleBreakpoint),
                MenuItem::action("Resume", Resume),
                MenuItem::action("Step Over", StepOver),
                MenuItem::action("Step Into", StepInto),
                MenuItem::action("Step Out", StepOut),
                MenuItem::separator(),
                MenuItem::action("Edit Configurations", EditRunConfigurations),
                MenuItem::action("Edit Debuggers", EditDebuggers),
            ],
        ),
        menu(
            "Git",
            vec![
                MenuItem::action("Show Changes", ToggleGitPanel),
                MenuItem::action("Switch Branch…", SwitchBranch),
            ],
        ),
    ]);
}

fn menu(name: &str, items: Vec<MenuItem>) -> Menu {
    Menu {
        name: name.to_string().into(),
        items,
        disabled: false,
    }
}
