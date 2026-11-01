//! The one window: open files in tabs, with the project's files in a
//! sidebar.

mod agent;
mod agents_file;
mod brackets;
mod breakpoints;
mod code_menu;
mod commands;
mod completions;
mod debug;
mod definitions;
mod diagnostics;
mod file_watch;
mod find;
mod find_panel;
mod format;
mod git;
mod go_to_file;
mod go_to_line;
mod home;
mod hover;
mod line_edits;
mod live;
mod lsp;
mod map;
mod map_agent;
mod mode;
mod navigation;
mod occurrences;
mod planner;
mod preferences;
mod quick_fix;
mod rename;
mod run;
mod selection_button;
mod session;
mod sidebar;
mod snippets;
mod status_bar;
mod tabs;

use std::path::{Path, PathBuf};

use gpui_kit::component::input::Editor;
use gpui_kit::component::{ActiveTheme as _, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_commands::CommandPalette;
use jig_editor::EditorHandle;

use commands::{CommandRun, Note};
use jig_editing::document::Document;
use jig_files::file_tree::FileTree;
use jig_kit::theme;
use jig_settings::{self as settings, Settings};
use mode::Mode;
use tabs::Tab;

pub use jig_preferences::settings_window::{
    EditAgents, EditDebuggers, EditFormatters, EditLanguageServers, EditModelConfig,
};
pub use jig_search::quick_open::GoToFile;

mod projects;
mod render;
mod saving;

actions!(
    jig,
    [
        Quit,
        Open,
        Save,
        SaveAs,
        CloseWindow,
        OpenCommand,
        SendSelectionToAgent,
        OpenAgentConversations,
        EditAgentsFile,
        ToggleSidebar,
        FocusFileTree,
        Find,
        FindAndReplace,
        FindInFiles,
        NewFile,
        CloseTab,
        NextTab,
        PreviousTab,
        RunSelected,
        ChooseRunConfiguration,
        StopRun,
        ToggleRunPanel,
        EditRunConfigurations,
        DebugSelected,
        ToggleBreakpoint,
        Resume,
        StepOver,
        StepInto,
        StepOut,
        ToggleGitPanel,
        SwitchBranch,
        NewBranch,
        RenameSymbol,
        FindReferences,
        NextProblem,
        PreviousProblem,
        ShowHover,
        ShowCompletions,
        ToggleLineComment,
        FormatDocument,
        MoveLineUp,
        MoveLineDown,
        DuplicateLine,
        DeleteLine,
        SelectLine,
        InsertLineBelow,
        InsertLineAbove,
        GoToLine,
        ZoomIn,
        ZoomOut,
        ResetZoom,
        ToggleTaskPlanner,
        NewTask,
        GoBack,
        GoForward,
        ShowMap,
        ShowCode,
        ShowAgent,
        ShowPlanner,
        GoToMap,
        IndexDomains,
        UpdateDomains,
        ExplainLevel
    ]
);

/// Switch to the tab at this index (⌃1…9, Alt-1…9 off macOS).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = jig, no_json)]
pub struct ActivateTab(pub usize);

pub(crate) const CONTEXT: &str = "Workspace";

/// Sidebar widths the user can drag between.
const MIN_SIDEBAR_WIDTH: f32 = 160.;
const MAX_SIDEBAR_WIDTH: f32 = 480.;

/// The unified title bar; `main.rs` centres the traffic lights in it.
const TITLE_BAR_HEIGHT: f32 = 46.;
/// Room kept clear for the traffic lights when the sidebar is hidden.
const TRAFFIC_LIGHTS_WIDTH: f32 = 84.;

pub struct Workspace {
    /// Never empty: closing the last tab leaves an Untitled one.
    tabs: Vec<Tab>,
    active: usize,
    tab_scroll: ScrollHandle,
    /// The project's files. `None` until a file or folder is open.
    tree: Option<ProjectTree>,
    sidebar: sidebar::SidebarState,
    /// A start page is showing in place of the editor, because no file is
    /// open: the home page, or with a project open, the project's. The one
    /// tab behind it is blank.
    home: bool,
    home_focus: FocusHandle,
    palette: Option<OpenPalette>,
    /// The agent's default model when last seen, to tell when Settings
    /// changes it.
    model_defaults: Option<Option<Option<String>>>,
    /// How many times the palette has opened, for the Agent button's glow.
    palette_openings: usize,
    /// What the window keeps for the agent between conversations.
    agent: agent::AgentState,
    /// The agent's earlier conversations and commands for the project.
    catalog: Entity<agent::catalog::ConversationCatalog>,
    /// Go to File's panel, and the files it offers first.
    quick_open: Entity<go_to_file::GoToFilePanel>,
    /// Find in Files' panel, and the last search.
    find_in_files: Entity<find::FindInFilesPanel>,
    /// Rename Symbol's field.
    rename: Entity<rename::RenameField>,
    /// Go to Line's field.
    go_to_line: Entity<go_to_line::GoToLineField>,
    /// Find and Replace in the current file.
    find_panel: Entity<find_panel::FindInFile>,
    /// The project's files as last walked, for Go to File to show at once.
    file_index: Option<go_to_file::FileIndex>,
    /// The AI providers file, `~/.config/jig/config.toml`.
    config_path: Option<PathBuf>,
    /// The settings as last applied, to tell what a change touched.
    settings: Settings,
    /// A note beside the code, such as why nothing can run yet.
    note: Option<Note>,
    /// The agent conversation. Its `agent` is set once it's set up.
    chat: Option<CommandRun>,
    /// The button beside the selection that opens the agent input.
    selection_button: Entity<selection_button::SelectionButton>,
    next_run_id: u64,
    /// Run configurations, the one ⌃R runs, and the picker.
    run_configs: Entity<run::configurations::RunConfigurations>,
    /// The configuration running.
    runs: run::RunState,
    /// What it printed, in the panel under the editor.
    run_output: Entity<run::output::RunOutput>,
    breakpoints: breakpoints::Breakpoints,
    git: git::GitState,
    /// The branch picker.
    branch_switcher: Entity<git::branches::BranchSwitcher>,
    /// The popup with a change's old lines.
    hunk_popup: Entity<git::hunk::HunkPopup>,
    /// The project's tasks, in the sidebar's Tasks view.
    planner: planner::PlannerState,
    /// New Task or Edit Task.
    task_form: Entity<planner::TaskFormSheet>,
    /// Where jumps came from, for Go Back and Go Forward.
    history: navigation::History,
    /// The logpoints that light the Map's functions up as the program
    /// runs.
    live: live::LivePoints,
    /// What shows in the editor's place: the code, the Planner or the Map.
    mode: Mode,
    /// The Map, once it has shown.
    map: Option<map::MapPane>,
    /// How far a pinch in the code has gone out, towards the Map.
    pinched: f32,
    file_watch: file_watch::FileWatch,
    /// A save waiting on its formatter.
    pending_save: Option<format::PendingSave>,
    /// A note in the status bar, such as why a save wasn't formatted.
    status_note: Entity<status_bar::StatusMessage>,
}

struct ProjectTree {
    view: Entity<FileTree>,
    _events: Subscription,
}

struct OpenPalette {
    view: Entity<CommandPalette>,
    /// Window position of the palette's top-left corner, fixed when it opens.
    anchor: Point<Pixels>,
    /// Which opening it is, so the Agent button's glow plays afresh.
    serial: usize,
    _events: Subscription,
}

impl Workspace {
    pub fn new(path: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // `jig .` or `jig ../x`: everything after works with full paths, as
        // language servers need them.
        let path = path.map(|path| {
            path.canonicalize()
                .or_else(|_| std::path::absolute(&path))
                .unwrap_or(path)
        });
        let folder = path.as_deref().filter(|path| path.is_dir());
        let (document, error) = match path.as_deref().filter(|_| folder.is_none()) {
            Some(path) => match Document::open(path) {
                Ok(document) => (document, None),
                Err(error) => (Document::default(), Some(error)),
            },
            None => (Document::default(), None),
        };
        let settings = settings::get(cx);
        let workspace = cx.entity();
        let selection_button = cx.new(|cx| selection_button::SelectionButton::new(&workspace, cx));
        let go_to_line = cx.new(|_| go_to_line::GoToLineField::default());
        cx.subscribe_in(&go_to_line, window, Self::on_go_to_line_event)
            .detach();
        let rename = cx.new(|_| rename::RenameField::default());
        cx.subscribe_in(&rename, window, Self::on_rename_event)
            .detach();
        let run_configs = cx.new(|_| run::configurations::RunConfigurations::default());
        cx.subscribe_in(&run_configs, window, Self::on_run_configurations_event)
            .detach();
        // The title bar names the configuration ⌃R runs.
        cx.observe(&run_configs, |_, _, cx| cx.notify()).detach();
        let run_output = cx.new(run::output::RunOutput::new);
        cx.subscribe_in(&run_output, window, Self::on_run_output_event)
            .detach();
        let branch_switcher = cx.new(|_| git::branches::BranchSwitcher::default());
        cx.subscribe_in(&branch_switcher, window, Self::on_branch_switcher_event)
            .detach();
        let hunk_popup = cx.new(|_| git::hunk::HunkPopup::default());
        cx.subscribe_in(&hunk_popup, window, Self::revert_hunk)
            .detach();
        let quick_open = cx.new(|_| go_to_file::GoToFilePanel::default());
        cx.subscribe_in(&quick_open, window, Self::on_go_to_file_event)
            .detach();
        let find_in_files = cx.new(|_| find::FindInFilesPanel::default());
        cx.subscribe_in(&find_in_files, window, Self::on_find_in_files_event)
            .detach();
        let task_form = cx.new(|_| planner::TaskFormSheet::default());
        cx.subscribe_in(&task_form, window, Self::on_task_form_event)
            .detach();
        let catalog = cx.new(|_| agent::catalog::ConversationCatalog::default());
        cx.subscribe(&catalog, Self::on_catalog_loaded).detach();
        // Agent mode lists what it holds.
        cx.observe(&catalog, |_, _, cx| cx.notify()).detach();
        let status_note = cx.new(|_| status_bar::StatusMessage::default());
        // Whether the status bar has a message decides what it holds.
        cx.observe(&status_note, |_, _, cx| cx.notify()).detach();
        let find_panel = cx.new(|_| find_panel::FindInFile::default());
        cx.subscribe_in(&find_panel, window, Self::on_find_dismissed)
            .detach();
        let mut this = Self {
            selection_button,
            tabs: Vec::new(),
            active: 0,
            tab_scroll: ScrollHandle::new(),
            tree: None,
            sidebar: sidebar::SidebarState::default(),
            home: path.is_none() || folder.is_some(),
            home_focus: cx.focus_handle(),
            palette: None,
            palette_openings: 0,
            model_defaults: None,
            agent: agent::AgentState::default(),
            catalog,
            quick_open,
            find_in_files,
            rename,
            go_to_line,
            find_panel,
            file_index: None,
            config_path: jig_ai::Config::user_path(),
            settings,
            note: None,
            chat: None,
            next_run_id: 0,
            run_configs,
            runs: run::RunState::new(),
            run_output,
            breakpoints: breakpoints::Breakpoints::load(),
            git: Default::default(),
            branch_switcher,
            hunk_popup,
            planner: Default::default(),
            task_form,
            history: Default::default(),
            mode: Mode::Code,
            map: None,
            pinched: 0.,
            live: Default::default(),
            file_watch: file_watch::FileWatch::start(window, cx),
            pending_save: None,
            status_note,
        };
        let tab = this.new_tab(document, window, cx);
        if this.home {
            this.home_focus.focus(window, cx);
        } else {
            tab.editor.focus(window, cx);
        }
        this.tabs.push(tab);
        if let Some(folder) = folder {
            this.open_folder(folder, window, cx);
        } else if let Some(file) = this.document().path.clone() {
            this.show_in_tree(&file, window, cx);
        }
        this.update_title(window);
        jig_kit::theme::sync(window, cx);
        cx.observe_window_appearance(window, |_, window, cx| jig_kit::theme::sync(window, cx))
            .detach();
        cx.observe_global_in::<settings::AppSettings>(window, Self::apply_settings)
            .detach();
        cx.observe_global::<jig_lsp::lsp::ServersChanged>(|this, cx| {
            this.reopen_in_servers(false, cx)
        })
        .detach();
        cx.observe_global::<jig_agent::providers::Providers>(|this, cx| {
            this.reload_provider(cx);
            // Model lists arriving fill in an open palette's chip.
            this.refresh_palette_models(cx);
        })
        .detach();
        cx.observe_global::<jig_lsp::lsp::diagnostics::Diagnostics>(Self::problems_reported)
            .detach();
        cx.observe_global::<jig_lsp::lsp::Failures>(Self::server_failed)
            .detach();
        // A downloaded update shows in the status bar.
        cx.observe_global::<jig_updates::Updates>(|_, cx| cx.notify())
            .detach();
        // A running configuration would outlive Jig otherwise.
        cx.on_app_quit(|this, _| {
            this.stop_process();
            async {}
        })
        .detach();
        // Pick up files added, removed or changed outside Jig.
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.reload_changed_files(None, window, cx);
                this.refresh_tree(cx);
                this.refresh_git(window, cx);
                this.move_to_current_servers(cx);
            }
        })
        .detach();

        window.on_window_should_close(cx, {
            let workspace = cx.entity().downgrade();
            move |window, cx| {
                workspace
                    .update(cx, |this, cx| this.confirm_close(window, cx))
                    .unwrap_or(true)
            }
        });

        if let Some(error) = error {
            this.show_error(&format!("{error:#}"), window, cx);
        }
        this
    }

    fn update_title(&self, window: &mut Window) {
        if self.home {
            window.set_window_title("Jig");
            window.set_window_edited(false);
            window.set_document_path(None);
            return;
        }
        let title = self.document().title();
        let marker = if self.tab().dirty { " •" } else { "" };
        window.set_window_title(&format!("{title}{marker}"));
        window.set_window_edited(self.any_dirty());
        window.set_document_path(self.document().path.as_deref());
    }

    fn show_error(&self, message: &str, window: &mut Window, cx: &mut Context<Self>) {
        // The only answer is OK, so nothing waits on it.
        drop(window.prompt(PromptLevel::Critical, "Jig", Some(message), &["OK"], cx));
    }
}

/// Jig's bindings. The ones Settings can change are in
/// [`crate::shortcuts`], here with their defaults.
pub fn key_bindings() -> Vec<KeyBinding> {
    crate::shortcuts::bindings(&Default::default())
        .into_iter()
        .chain(run::output::key_bindings())
        .chain(jig_files::file_tree::key_bindings())
        .chain(jig_search::find_in_files::key_bindings())
        .chain(jig_editing::find_panel::key_bindings())
        .chain(jig_map::key_bindings())
        .collect()
}

#[cfg(test)]
mod tests;
