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
    quick_open: Option<go_to_file::OpenQuickOpen>,
    find_in_files: Option<find::OpenFindInFiles>,
    /// Rename Symbol's field.
    rename: Entity<rename::RenameField>,
    /// Go to Line's field.
    go_to_line: Entity<go_to_line::GoToLineField>,
    last_find: find::LastFind,
    /// Find and Replace in the current file.
    find_panel: Entity<find_panel::FindInFile>,
    /// The project's files as last walked, for Go to File to show at once.
    file_index: Option<go_to_file::FileIndex>,
    /// Files shown in this window, most recent first, for Go to File.
    recent_files: Vec<PathBuf>,
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
    /// The configuration running, and its output.
    runs: run::RunState,
    breakpoints: breakpoints::Breakpoints,
    git: git::GitState,
    /// The project's tasks, in the sidebar's Tasks view.
    planner: planner::PlannerState,
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
            quick_open: None,
            find_in_files: None,
            rename,
            go_to_line,
            last_find: Default::default(),
            find_panel,
            file_index: None,
            recent_files: Vec::new(),
            config_path: jig_ai::Config::user_path(),
            settings,
            note: None,
            chat: None,
            next_run_id: 0,
            run_configs,
            runs: run::RunState::new(cx),
            breakpoints: breakpoints::Breakpoints::load(),
            git: Default::default(),
            planner: Default::default(),
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

    /// Run `then` now if there are no unsaved changes, otherwise only after
    /// the user agrees to discard them. Checks the tabs `only`, or every tab.
    fn when_discard_ok(
        &mut self,
        only: Option<&[EntityId]>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let dirty: Vec<String> = self
            .tabs
            .iter()
            .filter(|tab| tab.dirty && only.is_none_or(|ids| ids.contains(&tab.id())))
            .map(|tab| tab.document.title())
            .collect();
        let detail = match dirty.as_slice() {
            [] => {
                then(self, window, cx);
                return;
            }
            [title] => format!("{title} has unsaved changes."),
            titles => format!("{} files have unsaved changes.", titles.len()),
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            "Discard unsaved changes?",
            Some(&detail),
            &["Discard", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| {
            if answer.await == Ok(0) {
                this.update_in(cx, |this, window, cx| then(this, window, cx))
                    .ok();
            }
        })
        .detach();
    }

    fn confirm_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.any_dirty() {
            return true;
        }
        self.close_after_discard(window, cx);
        false
    }

    fn close_after_discard(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.when_discard_ok(None, window, cx, |this, window, _| {
            // So closing doesn't ask again.
            for tab in &mut this.tabs {
                tab.dirty = false;
            }
            window.remove_window();
        });
    }

    /// Ask for a folder, or with `files`, a file or folder, and open it.
    fn prompt_open(&mut self, files: bool, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            this.update_in(cx, |this, window, cx| {
                if path.is_dir() {
                    this.open_project(&path, window, cx);
                } else {
                    this.open_file(&path, window, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Open the folder `dir` as the project. With another project already
    /// open, ask whether it replaces that one or gets a window of its own.
    pub(super) fn open_project(&mut self, dir: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let dir = tabs::canonical(dir);
        let Some(current) = self.project_root(cx).filter(|root| *root != dir) else {
            self.open_folder(&dir, window, cx);
            return;
        };
        let name = |path: &Path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string())
        };
        let detail = format!("This window has {} open.", name(&current));
        let answer = window.prompt(
            PromptLevel::Info,
            &format!("Open {} in this window or a new one?", name(&dir)),
            Some(&detail),
            &["This Window", "New Window", "Cancel"],
            cx,
        );
        cx.spawn_in(window, async move |this, cx| match answer.await {
            Ok(0) => {
                this.update_in(cx, |this, window, cx| this.replace_project(dir, window, cx))
                    .ok();
            }
            Ok(1) => {
                cx.update(|_, cx| {
                    if let Err(error) = crate::open_window(Some(dir), &[], cx) {
                        eprintln!("jig: the new window didn't open: {error:#}");
                    }
                })
                .ok();
            }
            _ => {}
        })
        .detach();
    }

    /// Close this window's project, tabs and run included, and open `dir`
    /// in its place.
    fn replace_project(&mut self, dir: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.when_discard_ok(None, window, cx, move |this, window, cx| {
            this.leave_tab(cx);
            this.stop_process();
            this.runs = run::RunState::new(cx);
            this.run_configs
                .update(cx, |configurations, cx| configurations.reset(cx));
            this.palette = None;
            this.quick_open = None;
            this.find_in_files = None;
            this.file_index = None;
            this.recent_files.clear();
            let tab = this.new_tab(Document::default(), window, cx);
            this.tabs = vec![tab];
            this.active = 0;
            this.home = true;
            this.tree = None;
            this.open_folder(&dir, window, cx);
            this.update_title(window);
        });
    }

    fn open(&mut self, _: &Open, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_open(true, window, cx);
    }

    fn save(&mut self, _: &Save, window: &mut Window, cx: &mut Context<Self>) {
        if self.home {
            return;
        }
        match self.document().path.clone() {
            Some(path) => self.save_tidied(path, window, cx),
            None => self.prompt_save_as(window, cx),
        }
    }

    fn save_as(&mut self, _: &SaveAs, window: &mut Window, cx: &mut Context<Self>) {
        if self.home {
            return;
        }
        self.prompt_save_as(window, cx);
    }

    fn prompt_save_as(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let dir = self
            .document()
            .path
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        let name = self.document().title();
        let path = cx.prompt_for_new_path(&dir, Some(&name));
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(path))) = path.await else {
                return;
            };
            this.update_in(cx, |this, window, cx| this.save_tidied(path, window, cx))
                .ok();
        })
        .detach();
    }

    fn save_to(&mut self, path: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.editor().text(cx);
        let languages = &self.settings.languages;
        let language_changed =
            jig_languages::language_for(path, languages) != self.document().language(languages);
        if let Err(error) = self.tab_mut().document.save(path, &text) {
            self.show_error(&format!("Could not save: {error:#}"), window, cx);
            return;
        }
        if language_changed {
            // Rebuild so highlighting matches the new extension.
            self.reload_tab(path, window, cx);
        }
        self.tab_mut().dirty = false;
        self.lsp_saved(cx);
        self.update_title(window);
        if self.is_config_file() {
            jig_agent::providers::reload(cx);
        }
        self.run_file_saved(path, cx);
        self.refresh_git_panel(window, cx);
        self.remember_breakpoints(self.active, cx);
        // Save As may have added a file.
        self.refresh_tree(cx);
        cx.notify();
    }

    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.close_after_discard(window, cx);
    }

    fn quit(&mut self, _: &Quit, window: &mut Window, cx: &mut Context<Self>) {
        self.when_discard_ok(None, window, cx, |_, _, cx| cx.quit());
    }

    /// Install the downloaded update and start it on this window's project
    /// or file, as quitting would: unsaved changes are asked about first.
    fn restart_to_update(
        &mut self,
        _: &jig_updates::RestartToUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.when_discard_ok(None, window, cx, |this, window, cx| {
            let reopen = this
                .project_root(cx)
                .or_else(|| this.document().path.clone());
            if let Err(error) = jig_updates::restart(reopen, cx) {
                this.show_error(&error, window, cx);
            }
        });
    }
}

/// Jig's bindings. The ones Settings can change are in
/// [`crate::shortcuts`], here with their defaults.
pub fn key_bindings() -> Vec<KeyBinding> {
    crate::shortcuts::bindings(&Default::default())
        .into_iter()
        .chain(run::key_bindings())
        .chain(jig_files::file_tree::key_bindings())
        .chain(jig_search::find_in_files::key_bindings())
        .chain(jig_editing::find_panel::key_bindings())
        .chain(jig_map::key_bindings())
        .collect()
}

impl Workspace {
    /// The current tab's code, or the Planner or the Map in its place.
    fn render_code(&self, cx: &Context<Self>) -> AnyElement {
        if self.mode == Mode::Agent {
            return self.render_agent_mode(cx);
        }
        if self.mode == Mode::Planner {
            return div()
                .flex_1()
                .min_h_0()
                .child(self.render_planner_mode(cx))
                .into_any_element();
        }
        if let Some(view) = self.map_showing() {
            return div()
                .flex_1()
                .min_h_0()
                .child(view.clone())
                .into_any_element();
        }
        div()
            .flex_1()
            .min_h_0()
            .pl_2()
            .pr_3()
            .pt_1()
            .pb_2()
            // Line shortcuts bind here, not in every other input.
            .key_context("CodeEditor")
            .relative()
            .on_pinch(cx.listener(Self::pinch_code))
            .child(
                Editor::new(self.editor().state())
                    .bordered(false)
                    // The column behind it is the surface.
                    .bg(transparent_black())
                    // Locked while the agent's edit awaits review. The
                    // element re-applies this every frame.
                    .readonly(self.previewing())
                    .context_menu(self.code_menu(cx))
                    .size_full(),
            )
            .children(self.pinch_ruler(cx))
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        self.step_sidebar_motion(window);
        let sidebar = self.render_sidebar(cx);
        let title = self.render_title(cx);
        let tab_bar = self.render_tab_bar(cx);
        let root = v_flex();
        let root = self.agent_dock_drag_handlers(root, cx);
        self.run_panel_drag_handlers(self.sidebar_drag_handlers(root, cx), cx)
            .key_context(CONTEXT)
            .relative()
            .size_full()
            // No background here: on macOS the window is translucent and
            // the sidebar lets it show; the editor side paints its own.
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::restart_to_update))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_as))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::open_command))
            .on_action(cx.listener(Self::send_selection_to_agent))
            .on_action(cx.listener(Self::open_agent_conversations))
            .on_action(cx.listener(Self::edit_agents))
            .on_action(cx.listener(Self::edit_agents_file))
            .on_action(cx.listener(Self::edit_model_config))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::focus_file_tree))
            .on_action(cx.listener(Self::go_to_file))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::find_and_replace))
            .on_action(cx.listener(Self::find_in_files))
            .on_action(cx.listener(Self::new_file))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::activate_tab))
            .on_action(cx.listener(Self::run_selected))
            .on_action(cx.listener(Self::choose_run_configuration))
            .on_action(cx.listener(Self::stop_run))
            .on_action(cx.listener(Self::toggle_run_panel))
            .on_action(cx.listener(Self::edit_run_configurations))
            .on_action(cx.listener(Self::edit_debuggers))
            .on_action(cx.listener(Self::debug_selected))
            .on_action(cx.listener(Self::toggle_breakpoint))
            .on_action(cx.listener(Self::resume))
            .on_action(cx.listener(Self::step_over))
            .on_action(cx.listener(Self::step_into))
            .on_action(cx.listener(Self::step_out))
            .on_action(cx.listener(Self::toggle_git_panel))
            .on_action(cx.listener(Self::switch_branch))
            .on_action(cx.listener(Self::new_branch))
            .on_action(cx.listener(Self::rename_symbol))
            .on_action(cx.listener(Self::find_references))
            .on_action(cx.listener(Self::next_problem))
            .on_action(cx.listener(Self::previous_problem))
            .on_action(cx.listener(Self::show_hover))
            .on_action(cx.listener(Self::show_completions))
            .on_action(cx.listener(Self::toggle_line_comment))
            .on_action(cx.listener(Self::format_document))
            .on_action(cx.listener(Self::edit_formatters))
            .on_action(cx.listener(Self::edit_language_servers))
            .on_action(cx.listener(Self::move_line_up))
            .on_action(cx.listener(Self::move_line_down))
            .on_action(cx.listener(Self::duplicate_line))
            .on_action(cx.listener(Self::delete_line))
            .on_action(cx.listener(Self::select_line))
            .on_action(cx.listener(Self::insert_line_below))
            .on_action(cx.listener(Self::insert_line_above))
            .on_action(cx.listener(Self::go_to_line))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::reset_zoom))
            .on_action(cx.listener(Self::toggle_task_planner))
            .on_action(cx.listener(Self::new_task))
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_forward))
            .on_action(cx.listener(Self::show_map))
            .on_action(cx.listener(Self::show_code))
            .on_action(cx.listener(Self::show_agent))
            .on_action(cx.listener(Self::show_planner))
            .on_action(cx.listener(Self::go_to_map))
            .on_action(cx.listener(Self::index_domains))
            .on_action(cx.listener(Self::explain_level))
            .on_action(cx.listener(Self::update_domains))
            .capture_any_mouse_down(cx.listener(Self::navigation_button))
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_accept_enter))
            .capture_action(cx.listener(Self::on_accept_tab))
            .capture_action(cx.listener(Self::on_shift_tab))
            .capture_action(cx.listener(Self::on_accept_indent))
            .capture_action(cx.listener(Self::on_undo))
            .on_drag_move(cx.listener(Self::drag_agent_chat))
            .on_drag_move(cx.listener(Self::resize_agent_chat))
            .child(
                // One bar across the whole window, the sidebar below it:
                // the traffic lights, the title, and the command and run
                // controls. The tabs sit in a row below.
                TitleBar::new()
                    .h(px(TITLE_BAR_HEIGHT))
                    .pl_0()
                    .border_b_0()
                    .bg(transparent_black())
                    .child(
                        h_flex()
                            .size_full()
                            .bg(theme::editor_surface(cx))
                            .border_b_1()
                            .border_color(theme.title_bar_border)
                            .child(
                                h_flex()
                                    .flex_1()
                                    .flex_basis(px(0.))
                                    .min_w_0()
                                    .child(div().flex_none().w(px(TRAFFIC_LIGHTS_WIDTH)))
                                    .child(title),
                            )
                            .children(self.render_mode_switch(cx))
                            .child(
                                h_flex()
                                    .flex_1()
                                    .flex_basis(px(0.))
                                    .min_w_0()
                                    .justify_end()
                                    .children(self.render_command_button(cx))
                                    .when(self.mode != Mode::Agent, |this| {
                                        this.children(self.render_run_controls(cx))
                                    })
                                    .child(div().flex_none().w(px(8.))),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .when(self.mode != Mode::Agent, |this| {
                        this.children(self.render_activity_bar(cx))
                            .children(sidebar)
                    })
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .bg(theme::editor_surface(cx))
                            .children(tab_bar)
                            .when(self.start_showing(), |this| {
                                this.child(div().flex_1().min_h_0().child(self.render_start(cx)))
                            })
                            .when(!self.start_showing(), |this| {
                                this.child(
                                    // The find panel floats over the code,
                                    // outside its key context.
                                    div()
                                        .relative()
                                        .flex_1()
                                        .min_h_0()
                                        .flex()
                                        .flex_col()
                                        .child(self.render_code(cx))
                                        .children(self.render_find_panel(cx)),
                                )
                            })
                            .when(self.mode != Mode::Agent, |this| {
                                this.children(self.render_run_panel(cx))
                            })
                            .when(!self.home && self.mode != Mode::Agent, |this| {
                                this.child(self.render_status_bar(cx))
                            }),
                    )
                    .children(self.render_pinned_agent_chat(cx)),
            )
            .when(self.mode != Mode::Agent, |this| {
                this.children(self.render_sidebar_handle(cx))
            })
            .children(self.render_floating_settings(cx))
            .when_some(self.palette.as_ref(), |this, palette| {
                this.child(deferred(
                    anchored()
                        .position(palette.anchor)
                        .snap_to_window_with_margin(px(8.))
                        .child(palette.view.clone()),
                ))
            })
            .children(self.render_task_form(window))
            .children(self.render_quick_open(window))
            .children(self.render_find_in_files(window))
            .children(self.render_rename(cx))
            .children(self.render_go_to_line(cx))
            .children(self.render_run_picker(cx))
            .children(self.render_branch_picker(window))
            .children(self.render_hunk_popup(cx))
            .when(self.mode == Mode::Code && !self.home, |this| {
                this.child(self.tab().problems.hover.clone())
                    .child(self.tab().hover.clone())
            })
            .when(self.mode == Mode::Code, |this| {
                this.child(self.selection_button.clone())
            })
            // The agent conversation, unless it's docked, and a note.
            .when_some(
                self.chat
                    .as_ref()
                    .filter(|_| !self.floating_agent_chat_pinned())
                    .zip(self.render_agent_chat(cx)),
                |this, (chat, conversation)| {
                    this.child(deferred(
                        anchored()
                            .position(chat.anchor)
                            .snap_to_window_with_margin(px(8.))
                            .child(conversation),
                    ))
                },
            )
            .when_some(self.note.as_ref(), |this, note| {
                this.child(deferred(
                    anchored()
                        .position(note.anchor)
                        .snap_to_window_with_margin(px(8.))
                        .child(note.bubble.clone()),
                ))
            })
    }
}

#[cfg(test)]
mod tests;
