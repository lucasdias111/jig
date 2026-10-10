//! The one window: open files in tabs, with the project's files in a
//! sidebar.

mod agent;
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
mod lsp;
mod preferences;
mod rename;
mod run;
mod selection_button;
mod session;
mod sidebar;
mod snippets;
mod status_bar;
mod tabs;
mod user_commands;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::Editor;
use gpui_kit::component::{ActiveTheme as _, TitleBar, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use jig_ai::Provider;
use jig_commands::{CommandPalette, Preset, presets};
use jig_editor::EditorHandle;

use crate::document::Document;
use crate::file_tree::FileTree;
use crate::settings::{self, Settings};
use crate::theme;
use commands::CommandRun;
use tabs::Tab;

actions!(
    jig,
    [
        Quit,
        Open,
        Save,
        SaveAs,
        CloseWindow,
        OpenCommand,
        AddCommand,
        EditCommands,
        EditAgentsFile,
        EditModelConfig,
        ToggleSidebar,
        FocusFileTree,
        GoToFile,
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
        EditDebuggers,
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
        ToggleLineComment,
        FormatDocument,
        EditFormatters,
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
        ResetZoom
    ]
);

/// Switch to the tab at this index (Cmd+1…9).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = jig, no_json)]
pub struct ActivateTab(pub usize);

const CONTEXT: &str = "Workspace";

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
    sidebar_open: bool,
    sidebar_view: sidebar::SidebarView,
    sidebar_motion: Option<sidebar::SidebarMotion>,
    sidebar_width: Pixels,
    /// Set while the sidebar's edge is being dragged.
    resizing_sidebar: bool,
    /// A start page is showing in place of the editor, because no file is
    /// open: the home page, or with a project open, the project's. The one
    /// tab behind it is blank.
    home: bool,
    home_focus: FocusHandle,
    presets: Rc<Vec<Preset>>,
    palette: Option<OpenPalette>,
    quick_open: Option<go_to_file::OpenQuickOpen>,
    find_in_files: Option<find::OpenFindInFiles>,
    /// Rename Symbol's field, while it's open.
    rename: Option<rename::OpenRename>,
    /// Go to Line's field, while it's open.
    go_to_line: Option<go_to_line::OpenGoToLine>,
    last_find: find::LastFind,
    /// Find and Replace in the current file, while it's open.
    find_panel: Option<find_panel::OpenFindPanel>,
    /// The panel's last query, to start the next ⌘F from.
    last_find_query: String,
    /// The project's files as last walked, for Go to File to show at once.
    file_index: Option<go_to_file::FileIndex>,
    /// Files shown in this window, most recent first, for Go to File.
    recent_files: Vec<PathBuf>,
    new_command: Option<user_commands::OpenForm>,
    /// The user's jigs file, `~/.config/jig/jigs.toml`.
    commands_path: Option<PathBuf>,
    /// The configured model, or why it couldn't be set up.
    provider: Result<Arc<dyn Provider>, String>,
    /// The AI providers file, `~/.config/jig/config.toml`.
    config_path: Option<PathBuf>,
    /// The settings as last applied, to tell what a change touched.
    settings: Settings,
    run: Option<CommandRun>,
    /// The button beside the selection that opens the command input.
    selection_button: Entity<selection_button::SelectionButton>,
    next_run_id: u64,
    /// Run configurations, and the one running.
    runs: run::RunState,
    breakpoints: breakpoints::Breakpoints,
    git: git::GitState,
    file_watch: file_watch::FileWatch,
    /// A save waiting on its formatter.
    pending_save: Option<format::PendingSave>,
    /// A note in the status bar, such as why a save wasn't formatted.
    status_note: Option<format::StatusNote>,
}

struct ProjectTree {
    view: Entity<FileTree>,
    _events: Subscription,
}

struct OpenPalette {
    view: Entity<CommandPalette>,
    /// Window position of the palette's top-left corner, fixed when it opens.
    anchor: Point<Pixels>,
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
        let (presets, presets_error) = match presets::load(presets::user_jigs_path().as_deref()) {
            Ok(presets) => (presets, None),
            Err(error) => (presets::defaults(), Some(error)),
        };
        let presets = preferences::visible_presets(presets, &settings);
        let workspace = cx.entity();
        let selection_button = cx.new(|cx| selection_button::SelectionButton::new(&workspace, cx));
        let mut this = Self {
            selection_button,
            tabs: Vec::new(),
            active: 0,
            tab_scroll: ScrollHandle::new(),
            tree: None,
            sidebar_open: false,
            sidebar_view: Default::default(),
            sidebar_motion: None,
            sidebar_width: px(240.),
            resizing_sidebar: false,
            home: path.is_none() || folder.is_some(),
            home_focus: cx.focus_handle(),
            presets: Rc::new(presets),
            palette: None,
            quick_open: None,
            find_in_files: None,
            rename: None,
            go_to_line: None,
            last_find: Default::default(),
            find_panel: None,
            last_find_query: String::new(),
            file_index: None,
            recent_files: Vec::new(),
            new_command: None,
            commands_path: presets::user_jigs_path(),
            provider: crate::providers::build(cx),
            config_path: jig_ai::Config::user_path(),
            settings,
            run: None,
            next_run_id: 0,
            runs: run::RunState::new(cx),
            breakpoints: breakpoints::Breakpoints::load(),
            git: Default::default(),
            file_watch: file_watch::FileWatch::start(window, cx),
            pending_save: None,
            status_note: None,
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
        crate::theme::sync(window, cx);
        cx.observe_window_appearance(window, |_, window, cx| crate::theme::sync(window, cx))
            .detach();
        cx.observe_global_in::<settings::AppSettings>(window, Self::apply_settings)
            .detach();
        cx.observe_global::<crate::providers::Providers>(|this, cx| this.reload_provider(cx))
            .detach();
        cx.observe_global::<crate::lsp::diagnostics::Diagnostics>(Self::problems_reported)
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
            }
        })
        .detach();
        if let Some(error) = presets_error {
            this.show_error(&format!("Using the built-in jigs. {error:#}"), window, cx);
        }

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
            crate::languages::language_for(path, languages) != self.document().language(languages);
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
        if self.is_commands_file() {
            self.reload_presets(window, cx);
        }
        if self.is_config_file() {
            crate::providers::reload(cx);
        }
        self.run_file_saved(path, window, cx);
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
}

/// Jig's bindings. The ones Settings can change are in
/// [`crate::shortcuts`], here with their defaults.
pub fn key_bindings() -> Vec<KeyBinding> {
    crate::shortcuts::bindings(&Default::default())
        .into_iter()
        // Scoped to the workspace so that forms using Cmd+digits keep them.
        .chain(
            (1..=9).map(|n| {
                KeyBinding::new(&format!("secondary-{n}"), ActivateTab(n - 1), Some(CONTEXT))
            }),
        )
        .chain(run::key_bindings())
        .chain(crate::file_tree::key_bindings())
        .chain(crate::find_in_files::key_bindings())
        .chain(crate::find_panel::key_bindings())
        .chain(jig_commands::new_command::key_bindings())
        .collect()
}

impl Workspace {
    /// The current tab's code.
    fn render_code(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .pl_2()
            .pr_3()
            .pt_1()
            .pb_2()
            // Line shortcuts bind here, not in every other input.
            .key_context("CodeEditor")
            .child(
                Editor::new(self.editor().state())
                    .bordered(false)
                    // The column behind it is the surface.
                    .bg(transparent_black())
                    // Locked while a command's change awaits review. The
                    // element re-applies this every frame.
                    .readonly(self.previewing())
                    .context_menu(self.code_menu(cx))
                    .size_full(),
            )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let theme = cx.theme();
        self.step_sidebar_motion(window);
        let sidebar = self.render_sidebar(cx);
        let title = self.render_title(cx);
        let tab_bar = self.render_tab_bar(cx);
        let root = v_flex();
        self.run_panel_drag_handlers(self.sidebar_drag_handlers(root, cx), cx)
            .key_context(CONTEXT)
            .relative()
            .size_full()
            // No background here: on macOS the window is translucent and
            // the sidebar lets it show; the editor side paints its own.
            .on_action(cx.listener(Self::quit))
            .on_action(cx.listener(Self::open))
            .on_action(cx.listener(Self::save))
            .on_action(cx.listener(Self::save_as))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::open_command))
            .on_action(cx.listener(Self::add_command))
            .on_action(cx.listener(Self::edit_commands))
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
            .on_action(cx.listener(Self::toggle_line_comment))
            .on_action(cx.listener(Self::format_document))
            .on_action(cx.listener(Self::edit_formatters))
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
                            .pl(px(TRAFFIC_LIGHTS_WIDTH))
                            .pr_2()
                            .bg(theme::editor_surface(cx))
                            .border_b_1()
                            .border_color(theme.title_bar_border)
                            .child(title)
                            .children(self.render_command_button(cx))
                            .children(self.render_run_controls(cx)),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .children(self.render_activity_bar(cx))
                    .children(sidebar)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .bg(theme::editor_surface(cx))
                            .children(tab_bar)
                            .when(self.home, |this| {
                                this.child(div().flex_1().min_h_0().child(self.render_start(cx)))
                            })
                            .when(!self.home, |this| {
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
                                        .children(self.render_find_panel()),
                                )
                            })
                            .children(self.render_run_panel(cx))
                            .when(!self.home, |this| this.child(self.render_status_bar(cx))),
                    ),
            )
            .children(self.render_sidebar_handle(cx))
            .children(self.render_floating_settings(cx))
            .when_some(self.new_command.as_ref(), |this, form| {
                // Centred near the top, like a sheet.
                let left = ((viewport.width - px(460.)) / 2.).max(px(8.));
                this.child(deferred(
                    anchored()
                        .position(gpui_kit::point(left, px(TITLE_BAR_HEIGHT + 8.)))
                        .snap_to_window_with_margin(px(8.))
                        .child(form.view.clone()),
                ))
            })
            .when_some(self.palette.as_ref(), |this, palette| {
                this.child(deferred(
                    anchored()
                        .position(palette.anchor)
                        .snap_to_window_with_margin(px(8.))
                        .child(palette.view.clone()),
                ))
            })
            .children(self.render_quick_open(window))
            .children(self.render_find_in_files(window))
            .children(self.render_rename())
            .children(self.render_go_to_line())
            .children(self.render_run_picker(window))
            .children(self.render_branch_picker(window))
            .children(self.render_hunk_popup(cx))
            .when(!self.home, |this| {
                this.child(self.tab().problems.hover.clone())
                    .child(self.tab().hover.clone())
            })
            .child(self.selection_button.clone())
            .when_some(self.run.as_ref(), |this, run| {
                let floating = self
                    .render_agent_chat(cx)
                    .unwrap_or_else(|| run.bubble.clone().into_any_element());
                this.child(deferred(
                    anchored()
                        .position(run.anchor)
                        .snap_to_window_with_margin(px(8.))
                        .child(floating),
                ))
            })
    }
}

#[cfg(test)]
mod tests;
