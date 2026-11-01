//! Every shortcut Settings > Keyboard can change, with its default keys and action.

use super::*;

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
    // ⌘. as in VS Code.
    shortcut!(
        "quick_fix",
        "Quick fix",
        Editing,
        ["secondary-."],
        IN_CODE,
        ToggleCodeActions
    ),
    shortcut!(
        "format_document",
        "Format document",
        Editing,
        ["alt-shift-f"],
        IN_CODE,
        FormatDocument
    ),
    // Also in any input, to win over GPUI Kit's own Search and Replace
    // bindings there.
    shortcut!(
        "find",
        "Find",
        Editing,
        ["secondary-f"],
        &[None, Some("Input")],
        Find
    ),
    // ⌘R as in IntelliJ too; elsewhere ⌃R is Run, so it's the usual ⌃H.
    shortcut!(
        "replace",
        "Find and replace",
        Editing,
        [
            "secondary-alt-f",
            if cfg!(target_os = "macos") {
                "cmd-r"
            } else {
                "ctrl-h"
            }
        ],
        &[None, Some("Input")],
        FindAndReplace
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
    // As in Xcode and VS Code on macOS. Elsewhere ⌃- is a smaller font, so
    // it's VS Code's Alt-arrows.
    shortcut!(
        "go_back",
        "Go back",
        Navigation,
        [if cfg!(target_os = "macos") {
            "ctrl--"
        } else {
            "alt-left"
        }],
        ANYWHERE,
        GoBack
    ),
    shortcut!(
        "go_forward",
        "Go forward",
        Navigation,
        [if cfg!(target_os = "macos") {
            "ctrl-shift--"
        } else {
            "alt-right"
        }],
        ANYWHERE,
        GoForward
    ),
    // F8 alone is the debugger's step over.
    shortcut!(
        "next_problem",
        "Next problem",
        Navigation,
        ["alt-f8"],
        ANYWHERE,
        NextProblem
    ),
    shortcut!(
        "previous_problem",
        "Previous problem",
        Navigation,
        ["alt-shift-f8"],
        ANYWHERE,
        PreviousProblem
    ),
    // ⌘I as in VS Code's ⌘K ⌘I, without the ⌘K that asks the agent here.
    shortcut!(
        "show_hover",
        "Show hover",
        Navigation,
        ["secondary-i"],
        IN_CODE,
        ShowHover
    ),
    // ⌃Space everywhere, as in most editors; on a Mac it can clash with
    // switching input sources, and Settings can change it.
    shortcut!(
        "show_completions",
        "Show completions",
        Navigation,
        ["ctrl-space"],
        IN_CODE,
        ShowCompletions
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
    tab_shortcut::<0>("tab_1", "Tab 1"),
    tab_shortcut::<1>("tab_2", "Tab 2"),
    tab_shortcut::<2>("tab_3", "Tab 3"),
    tab_shortcut::<3>("tab_4", "Tab 4"),
    tab_shortcut::<4>("tab_5", "Tab 5"),
    tab_shortcut::<5>("tab_6", "Tab 6"),
    tab_shortcut::<6>("tab_7", "Tab 7"),
    tab_shortcut::<7>("tab_8", "Tab 8"),
    tab_shortcut::<8>("tab_9", "Tab 9"),
    // Agent.
    shortcut!(
        // Named before jigs went, so changed keys carry over.
        "run_jig",
        "Ask the agent",
        Agent,
        ["secondary-k"],
        ANYWHERE,
        OpenCommand
    ),
    shortcut!(
        "send_to_agent",
        "Send the selection to the agent",
        Agent,
        ["secondary-shift-l"],
        IN_CODE,
        SendSelectionToAgent
    ),
    shortcut!(
        "pick_model",
        "Pick agent and model (in the agent input)",
        Agent,
        ["alt-m"],
        &[Some(jig_commands::palette::CONTEXT)],
        jig_commands::palette::PickModel
    ),
    shortcut!(
        "agent_conversations",
        "Agent conversations",
        Agent,
        ["secondary-alt-k"],
        ANYWHERE,
        OpenAgentConversations
    ),
    // View.
    shortcut!(
        "show_agent",
        "Agent",
        View,
        ["secondary-4"],
        IN_WORKSPACE,
        ShowAgent
    ),
    shortcut!(
        "show_code",
        "Code",
        View,
        ["secondary-1"],
        IN_WORKSPACE,
        ShowCode
    ),
    shortcut!(
        "show_planner",
        "Planner",
        View,
        ["secondary-2"],
        IN_WORKSPACE,
        ShowPlanner
    ),
    shortcut!(
        "go_to_map",
        "Map",
        View,
        ["secondary-3"],
        IN_WORKSPACE,
        GoToMap
    ),
    shortcut!(
        "toggle_sidebar",
        "Toggle sidebar",
        View,
        // ⌘B is the Map's.
        ["secondary-u"],
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
        "show_map",
        "Map, or back to the code from the Map",
        View,
        ["secondary-b"],
        ANYWHERE,
        ShowMap
    ),
    shortcut!(
        "toggle_task_planner",
        "Show tasks",
        View,
        ["ctrl-shift-t"],
        ANYWHERE,
        ToggleTaskPlanner
    ),
    shortcut!(
        "zoom_in",
        "Bigger code font, or zoom the Map in",
        View,
        // ⌘= where + is shifted (US), ⌘+ where it has its own key (Danish).
        ["secondary-=", "secondary-+"],
        ANYWHERE,
        ZoomIn
    ),
    shortcut!(
        "zoom_out",
        "Smaller code font, or zoom the Map out",
        View,
        ["secondary--"],
        ANYWHERE,
        ZoomOut
    ),
    shortcut!(
        "reset_zoom",
        "Default code font size, or the Map as it starts",
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
