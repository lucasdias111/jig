//! The macOS menu bar: Jig, File and Edit.

use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
use gpui_kit::*;

use crate::workspace::{
    AddCommand, CloseWindow, EditCommands, EditProjectRules, Open, OpenCommand, Quit, Save, SaveAs,
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
                    "Version {}\nAI commands at the cursor.",
                    env!("CARGO_PKG_VERSION")
                );
                // Nothing to do with the answer.
                drop(window.prompt(PromptLevel::Info, "Jig", Some(&detail), &["OK"], cx));
            })
            .ok();
    });

    cx.set_menus([
        // macOS titles this menu with the app's name.
        menu(
            "Jig",
            vec![
                MenuItem::action("About Jig", About),
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
                MenuItem::action("Open…", Open),
                MenuItem::separator(),
                MenuItem::action("Save", Save),
                MenuItem::action("Save As…", SaveAs),
                MenuItem::separator(),
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
                MenuItem::action("Run Command…", OpenCommand),
                MenuItem::action("Add Command…", AddCommand),
                MenuItem::action("Edit Commands File", EditCommands),
                MenuItem::action("Edit Project Rules (JIG.md)", EditProjectRules),
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
