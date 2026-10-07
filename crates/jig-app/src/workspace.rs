//! The one window: open files in tabs, with the project's files in a
//! sidebar.

mod agent;
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
mod tests {
    use gpui_kit::test::TestWindowExt;
    use std::path::Path;

    use gpui_kit::{
        AnyWindowHandle, AppContext, Bounds, Entity, Focusable, Point, TestAppContext,
        WindowBounds, WindowOptions, px, size,
    };
    use std::sync::Arc;

    use jig_ai::Provider;
    use jig_commands::{Bubble, ChatEntry};
    use jig_editor::EditorHandle;

    use super::{Workspace, key_bindings};

    /// The editor's move to the end of the text.
    const END_OF_FILE: &str = if cfg!(target_os = "macos") {
        "cmd-down"
    } else {
        "ctrl-end"
    };

    /// Answers every request with a fixed reply.
    struct FakeProvider(Result<&'static str, &'static str>);

    impl Provider for FakeProvider {
        fn complete(&self, _: &str, user: &str) -> anyhow::Result<String> {
            assert!(user.contains("<<<SELECTION>>>"), "the target is marked");
            self.0.map(str::to_string).map_err(|e| anyhow::anyhow!(e))
        }
    }

    fn use_provider(
        cx: &mut TestAppContext,
        workspace: &Entity<Workspace>,
        reply: Result<&'static str, &'static str>,
    ) {
        cx.update(|cx| {
            workspace.update(cx, |this, _| {
                this.provider = Ok(Arc::new(FakeProvider(reply)))
            })
        });
    }

    /// Select `range`, then run the preset matching `query` through Cmd+K.
    fn run_preset(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        workspace: &Entity<Workspace>,
        range: std::ops::Range<usize>,
        query: &str,
    ) {
        let workspace = workspace.clone();
        let query = query.to_string();
        step(cx, window, move |window, cx| {
            window.render_frame(cx);
            workspace.update(cx, |this, cx| {
                this.editor()
                    .state()
                    .update(cx, |s, cx| s.set_selected_range(range, cx))
            });
            window.press("secondary-k", cx);
        });
        step(cx, window, move |window, cx| {
            window.render_frame(cx);
            window.input(&query, cx);
            window.press("enter", cx);
        });
    }

    fn bubble(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Option<Bubble> {
        cx.update(|cx| {
            workspace
                .read(cx)
                .run
                .as_ref()
                .map(|run| run.bubble.clone())
        })
    }

    fn open(cx: &mut TestAppContext, path: &Path) -> (AnyWindowHandle, Entity<Workspace>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.bind_keys(key_bindings());
        });
        let path = path.to_path_buf();
        let handles = cx.update(|cx| {
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Point::default(),
                    size: size(px(800.), px(480.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Workspace::new(Some(path), window, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        handles
    }

    /// Run `f` in the window, then let subscriptions and effects settle.
    fn step(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        f: impl FnOnce(&mut gpui_kit::Window, &mut gpui_kit::App),
    ) {
        cx.update_window(window, |_, window, cx| f(window, cx))
            .unwrap();
        cx.run_until_parked();
    }

    /// A window as Jig opens with nothing to show.
    fn open_empty(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            cx.bind_keys(key_bindings());
        });
        let handles = cx.update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| Workspace::new(None, window, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        handles
    }

    #[gpui_kit::test]
    fn starts_on_the_home_page_and_new_file_leaves_it(cx: &mut TestAppContext) {
        let (window, workspace) = open_empty(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            let this = workspace.read(cx);
            assert!(this.home);
            assert!(this.home_focus.is_focused(window));
            window.press("secondary-k", cx);
        });
        assert!(
            cx.update(|cx| workspace.read(cx).palette.is_none()),
            "no commands without a file"
        );
        step(cx, window, |window, cx| window.press("secondary-n", cx));
        step(cx, window, |window, cx| {
            let this = workspace.read(cx);
            assert!(!this.home);
            assert_eq!(this.tabs.len(), 1, "the blank tab is the new file");
            assert!(this.editor().state().focus_handle(cx).is_focused(window));
        });
    }

    #[gpui_kit::test]
    fn a_project_with_no_file_open_shows_its_page(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("lib.rs"), ORIGINAL).unwrap();
        let (window, workspace) = open(cx, dir.path());
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(this.home, "no Untitled tab");
            assert_eq!(this.tabs.len(), 1);
        });
        assert!(tree_focused(cx, window, &workspace));

        // Open lib.rs from the tree, then close it.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        });
        assert!(!cx.update(|cx| workspace.read(cx).home));
        assert_eq!(text(cx, &workspace), ORIGINAL);
        step(cx, window, |window, cx| window.press("secondary-w", cx));
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(this.home, "back to the project's page");
            assert_eq!(this.tabs.len(), 1);
            assert!(this.document().path.is_none());
        });
        assert!(tree_focused(cx, window, &workspace));

        // Cmd+S has nothing to save there.
        step(cx, window, |window, cx| window.press("secondary-s", cx));
        assert!(!cx.has_pending_prompt());
    }

    #[gpui_kit::test]
    fn opened_folders_become_recent_projects(cx: &mut TestAppContext) {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let (window, workspace) = open_empty(cx);
        let ws = workspace.clone();
        let (a, b) = (first.path().to_path_buf(), second.path().to_path_buf());
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| {
                this.open_folder(&a, window, cx);
                this.open_folder(&b, window, cx);
            })
        });
        assert!(
            cx.update(|cx| workspace.read(cx).home),
            "on the project's page: no file is open yet"
        );
        assert_eq!(
            cx.update(|cx| crate::recent::get(cx)),
            [
                second.path().canonicalize().unwrap(),
                first.path().canonicalize().unwrap()
            ]
        );
    }

    #[gpui_kit::test]
    fn opening_another_project_asks_where(cx: &mut TestAppContext) {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::write(first.path().join("lib.rs"), ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &first.path().join("lib.rs"));
        let root = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).project_root(cx));
        let open_second = |cx: &mut TestAppContext| {
            let (ws, dir) = (workspace.clone(), second.path().to_path_buf());
            step(cx, window, move |window, cx| {
                ws.update(cx, |this, cx| this.open_project(&dir, window, cx))
            });
        };

        open_second(cx);
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(root(cx), Some(first.path().canonicalize().unwrap()));

        open_second(cx);
        cx.simulate_prompt_answer("New Window");
        cx.run_until_parked();
        assert_eq!(cx.update(|cx| cx.windows().len()), 2);
        assert_eq!(root(cx), Some(first.path().canonicalize().unwrap()));

        open_second(cx);
        cx.simulate_prompt_answer("This Window");
        cx.run_until_parked();
        assert_eq!(root(cx), Some(second.path().canonicalize().unwrap()));
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(this.home, "on the new project's page");
            assert_eq!(this.tabs.len(), 1);
            assert!(this.document().path.is_none(), "the old file is closed");
        });
    }

    #[gpui_kit::test]
    fn language_settings_rehighlight_open_tabs(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.ts");
        std::fs::write(&path, "const a = 1;\n").unwrap();
        let (window, workspace) = open(cx, &path);

        step(cx, window, |_, cx| {
            assert_eq!(workspace.read(cx).editor().language(cx), "typescript");
            crate::settings::update(cx, |s| s.languages.set_off("typescript", true));
        });
        step(cx, window, |_, cx| {
            assert_eq!(workspace.read(cx).editor().language(cx), "text");
            crate::settings::update(cx, |s| s.languages.set_off("typescript", false));
        });
        step(cx, window, |_, cx| {
            assert_eq!(workspace.read(cx).editor().language(cx), "typescript");
        });
    }

    #[gpui_kit::test]
    fn edit_marks_dirty_and_save_writes_file(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            assert_eq!(workspace.read(cx).editor().language(cx), "rust");
            assert!(!workspace.read(cx).tab().dirty);
            window.press(END_OF_FILE, cx);
            window.input("// x\n", cx);
        });
        step(cx, window, |window, cx| {
            assert!(
                workspace.read(cx).tab().dirty,
                "typing marks the buffer dirty"
            );
            window.press("secondary-s", cx);
        });
        step(cx, window, |_, cx| {
            assert!(
                !workspace.read(cx).tab().dirty,
                "saving clears the dirty flag"
            )
        });
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn a() {}\n// x\n");
    }

    fn reload_changed_files(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        workspace: &Entity<Workspace>,
    ) {
        step(cx, window, |window, cx| {
            workspace.update(cx, |this, cx| this.reload_changed_files(None, window, cx))
        });
    }

    #[gpui_kit::test]
    fn files_changed_on_disk_reload_keeping_the_cursor(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\nfn b() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);
        let b = "fn a() {}\n".len();
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(b..b, cx);
        });

        std::fs::write(&path, "fn first() {}\nfn a() {}\nfn b() {}\n").unwrap();
        reload_changed_files(cx, window, &workspace);
        assert_eq!(
            text(cx, &workspace),
            "fn first() {}\nfn a() {}\nfn b() {}\n"
        );
        let b = b + "fn first() {}\n".len();
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert_eq!(this.editor().selection(cx), b..b);
            assert!(!this.tab().dirty);
        });
    }

    #[gpui_kit::test]
    fn files_changed_on_disk_keep_unsaved_changes(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("x", cx);
        });

        std::fs::write(&path, "fn other() {}\n").unwrap();
        reload_changed_files(cx, window, &workspace);
        assert_eq!(text(cx, &workspace), "xfn a() {}\n");
        assert!(cx.update(|cx| workspace.read(cx).tab().dirty));
    }

    #[gpui_kit::test]
    fn undoing_back_to_saved_text_clears_dirty(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("x", cx);
        });
        step(cx, window, |window, cx| {
            assert!(workspace.read(cx).tab().dirty);
            window.press("secondary-z", cx);
        });
        step(cx, window, |_, cx| assert!(!workspace.read(cx).tab().dirty));
    }

    const DOCS_REPLY: &str =
        r#"{"replace": "/// Does a.\nfn a() {}", "message": "Added a doc comment."}"#;
    const ORIGINAL: &str = "fn a() {}\n";
    const DOCUMENTED: &str = "/// Does a.\nfn a() {}\n";

    /// Open `lib.rs`, run "Add docs" on `fn a() {}` and wait for the preview.
    fn preview_docs(
        cx: &mut TestAppContext,
    ) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(cx, &workspace, Ok(DOCS_REPLY));
        run_preset(cx, window, &workspace, 0..9, "docs");
        (dir, window, workspace)
    }

    fn text(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> String {
        cx.update(|cx| workspace.read(cx).editor().text(cx))
    }

    #[gpui_kit::test]
    fn reply_is_previewed_in_place(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| {
            let this = workspace.read(cx);
            assert!(
                this.palette.is_none(),
                "running a command closes the palette"
            );
            assert!(
                this.editor().state().focus_handle(cx).is_focused(window),
                "focus returns to the editor"
            );
            let run = this.run.as_ref().expect("a command ran");
            assert_eq!(run.target, 0..9);
            assert_eq!(
                run.preview.as_ref().unwrap().range,
                0..12,
                "only the added doc line, not the untouched function"
            );
            assert_eq!(
                run.bubble,
                Bubble::Preview {
                    message: "Added a doc comment.".into(),
                    removed: String::new(),
                    agent: false,
                },
                "nothing was removed"
            );
            assert_eq!(
                this.editor().highlighted_ranges(cx),
                vec![0..12],
                "only the new line is highlighted"
            );
            assert_eq!(
                this.editor().text(cx),
                DOCUMENTED,
                "the change is in the buffer"
            );
            assert!(
                !this.editor().state().read(cx).is_editable(),
                "and locked while under review"
            );
        });
    }

    #[gpui_kit::test]
    fn enter_accepts_and_one_undo_reverts(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| window.press("enter", cx));
        assert_eq!(bubble(cx, &workspace), None);
        assert_eq!(
            text(cx, &workspace),
            DOCUMENTED,
            "Enter keeps the change and inserts nothing"
        );
        assert!(cx.update(|cx| workspace.read(cx).editor().state().read(cx).is_editable()));

        step(cx, window, |window, cx| window.press("secondary-z", cx));
        assert_eq!(
            text(cx, &workspace),
            ORIGINAL,
            "one undo reverts the whole command"
        );
    }

    #[gpui_kit::test]
    fn typing_is_blocked_during_review(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("zzz", cx);
        });
        assert_eq!(text(cx, &workspace), DOCUMENTED);
        assert!(bubble(cx, &workspace).is_some_and(|b| b.is_preview()));
    }

    #[gpui_kit::test]
    fn tab_accepts(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| window.press("tab", cx));
        assert_eq!(bubble(cx, &workspace), None);
        assert_eq!(
            text(cx, &workspace),
            DOCUMENTED,
            "Tab keeps the change and inserts nothing"
        );
    }

    #[gpui_kit::test]
    fn escape_rejects(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| window.press("escape", cx));
        assert_eq!(bubble(cx, &workspace), None);
        assert_eq!(text(cx, &workspace), ORIGINAL);
        assert!(
            !cx.update(|cx| workspace.read(cx).tab().dirty),
            "back to the saved text"
        );

        // The editor is usable again.
        step(cx, window, |window, cx| window.input("x", cx));
        assert!(text(cx, &workspace).contains('x'));
    }

    #[gpui_kit::test]
    fn undo_during_review_rejects(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| window.press("secondary-z", cx));
        assert_eq!(bubble(cx, &workspace), None);
        assert_eq!(text(cx, &workspace), ORIGINAL);
    }

    #[gpui_kit::test]
    fn reject_keeps_earlier_edits(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(
            cx,
            &workspace,
            Ok(r#"{"replace": "fn b() {}", "message": "Renamed."}"#),
        );
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press(END_OF_FILE, cx);
            window.input("// kept\n", cx);
        });
        run_preset(cx, window, &workspace, 0..9, "simplify");
        assert_eq!(text(cx, &workspace), "fn b() {}\n// kept\n");
        step(cx, window, |window, cx| window.press("escape", cx));
        assert_eq!(
            text(cx, &workspace),
            "fn a() {}\n// kept\n",
            "only the command is undone"
        );
    }

    #[gpui_kit::test]
    fn unchanged_reply_only_shows_the_message(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(
            cx,
            &workspace,
            Ok(r#"{"replace": "fn a() {}", "message": "Defines an empty function a."}"#),
        );
        run_preset(cx, window, &workspace, 0..9, "explain");
        assert_eq!(
            bubble(cx, &workspace),
            Some(Bubble::Message("Defines an empty function a.".into()))
        );
        assert_eq!(text(cx, &workspace), ORIGINAL);
        assert!(cx.update(|cx| workspace.read(cx).editor().state().read(cx).is_editable()));
    }

    #[gpui_kit::test]
    fn new_command_accepts_pending_change(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| window.press("secondary-k", cx));
        let this_text = text(cx, &workspace);
        assert_eq!(this_text, DOCUMENTED);
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(this.palette.is_some());
            assert!(this.editor().state().read(cx).is_editable());
        });
    }

    #[gpui_kit::test]
    fn waiting_tints_the_target_until_cancelled(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(cx, &workspace, Ok(DOCS_REPLY));

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            workspace.update(cx, |this, cx| {
                this.editor()
                    .state()
                    .update(cx, |s, cx| s.set_selected_range(0..9, cx))
            });
            window.press("secondary-k", cx);
        });
        // Run without letting the request finish.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input("docs", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(this.run.as_ref().unwrap().bubble.is_running());
            assert_eq!(this.editor().highlighted_ranges(cx), vec![0..9]);
        });
        cx.update_window(window, |_, window, cx| window.press("escape", cx))
            .unwrap();
        cx.run_until_parked();
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(this.run.is_none());
            assert!(this.editor().highlighted_ranges(cx).is_empty());
            assert_eq!(
                this.editor().text(cx),
                ORIGINAL,
                "a cancelled request changes nothing"
            );
        });
    }

    #[gpui_kit::test]
    fn provider_error_shows_and_fades(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(cx, &workspace, Err("401 Unauthorized: invalid api key"));

        run_preset(cx, window, &workspace, 0..9, "docs");
        assert_eq!(
            bubble(cx, &workspace),
            Some(Bubble::Error("401 Unauthorized: invalid api key".into()))
        );
        cx.executor()
            .advance_clock(std::time::Duration::from_secs(5));
        cx.run_until_parked();
        assert_eq!(bubble(cx, &workspace), None);
    }

    #[gpui_kit::test]
    fn malformed_reply_is_an_error(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(cx, &workspace, Ok("Sure, here is the code: fn a() {}"));

        run_preset(cx, window, &workspace, 0..9, "docs");
        assert!(matches!(bubble(cx, &workspace), Some(Bubble::Error(_))));
        assert_eq!(
            cx.update(|cx| workspace.read(cx).editor().text(cx)),
            "fn a() {}\n"
        );
    }

    #[gpui_kit::test]
    fn reply_for_a_changed_buffer_is_discarded(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);
        use_provider(cx, &workspace, Ok(r#"{"replace": "x", "message": "m"}"#));

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            workspace.update(cx, |this, cx| {
                this.editor()
                    .state()
                    .update(cx, |s, cx| s.set_selected_range(0..9, cx))
            });
            window.press("secondary-k", cx);
        });
        // Start the command; its request is queued but has not run yet.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.input("docs", cx);
            window.press("enter", cx);
        })
        .unwrap();
        // Edit while the request is in flight, then let it finish.
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            assert!(ws.read(cx).run.as_ref().unwrap().bubble.is_running());
            ws.update(cx, |this, cx| {
                this.editor().apply_edit(10..10, "// typed\n", window, cx);
            });
        });
        match bubble(cx, &workspace) {
            Some(Bubble::Error(message)) => assert!(message.contains("changed"), "{message}"),
            other => panic!("expected an error, got {other:?}"),
        }
    }

    #[gpui_kit::test]
    fn missing_api_key_is_reported(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);
        cx.update(|cx| {
            workspace.update(cx, |this, _| {
                this.provider = Err("OPENCODE_API_KEY is not set.".into())
            })
        });

        run_preset(cx, window, &workspace, 0..9, "docs");
        assert_eq!(
            bubble(cx, &workspace),
            Some(Bubble::Error("OPENCODE_API_KEY is not set.".into()))
        );
    }

    #[gpui_kit::test]
    fn escape_closes_palette(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn a() {}\n").unwrap();
        let (window, workspace) = open(cx, &path);

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-k", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("escape", cx);
        });
        step(cx, window, |window, cx| {
            let this = workspace.read(cx);
            assert!(this.palette.is_none());
            assert!(this.run.is_none());
            assert!(this.editor().state().focus_handle(cx).is_focused(window));
        });
    }

    /// A workspace whose commands file lives in a temp dir.
    fn open_with_commands(
        cx: &mut TestAppContext,
    ) -> (
        tempfile::TempDir,
        AnyWindowHandle,
        Entity<Workspace>,
        std::path::PathBuf,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        let commands = dir.path().join("config").join("jigs.toml");
        let commands_for_ws = commands.clone();
        cx.update(|cx| workspace.update(cx, |this, _| this.commands_path = Some(commands_for_ws)));
        (dir, window, workspace, commands)
    }

    fn preset_names(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
        cx.update(|cx| {
            workspace
                .read(cx)
                .presets
                .iter()
                .map(|p| p.name.clone())
                .collect()
        })
    }

    #[gpui_kit::test]
    fn add_command_from_the_shortcut(cx: &mut TestAppContext) {
        let (_dir, window, workspace, commands) = open_with_commands(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-k", cx);
        });
        assert!(cx.update(|cx| workspace.read(cx).new_command.is_some()));
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("Create controller", cx);
            window.press("tab", cx);
            window.input("Insert a REST controller at the cursor.", cx);
            window.press("secondary-2", cx);
            window.press("secondary-enter", cx);
        });

        let source = std::fs::read_to_string(&commands).unwrap();
        let saved = jig_commands::presets::parse(&source).unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].name, "Create controller");
        assert_eq!(saved[0].prompt, "Insert a REST controller at the cursor.");
        assert_eq!(saved[0].scope, jig_commands::Scope::Cursor);
        assert!(!saved[0].agent, "quick unless chosen otherwise");
        assert!(
            preset_names(cx, &workspace).contains(&"Create controller".to_string()),
            "available in ⌘K at once"
        );
        step(cx, window, |window, cx| {
            let this = workspace.read(cx);
            assert!(this.new_command.is_none());
            assert!(this.editor().state().focus_handle(cx).is_focused(window));
            assert!(
                matches!(this.run.as_ref().map(|r| &r.bubble), Some(Bubble::Message(m)) if m.contains("Create controller"))
            );
            assert_eq!(
                this.editor().text(cx),
                ORIGINAL,
                "the file being edited is untouched"
            );
        });
    }

    #[gpui_kit::test]
    fn save_typed_instruction_as_command(cx: &mut TestAppContext) {
        let (_dir, window, workspace, commands) = open_with_commands(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-k", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("wrap this in a tokio task", cx);
            window.press("secondary-enter", cx);
        });
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(this.palette.is_none());
            assert!(
                this.new_command.is_some(),
                "the form replaces the command input"
            );
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("Spawn task", cx);
            window.press("secondary-enter", cx);
        });
        let saved =
            jig_commands::presets::parse(&std::fs::read_to_string(&commands).unwrap()).unwrap();
        assert_eq!(
            saved[0].prompt, "wrap this in a tokio task",
            "the typed text becomes the prompt"
        );
        assert_eq!(saved[0].scope, jig_commands::Scope::Selection);
    }

    #[gpui_kit::test]
    fn duplicate_name_keeps_the_form_open(cx: &mut TestAppContext) {
        let (_dir, window, workspace, commands) = open_with_commands(cx);
        std::fs::create_dir_all(commands.parent().unwrap()).unwrap();
        std::fs::write(&commands, "[[jig]]\nname = \"Mine\"\nprompt = \"p\"\n").unwrap();
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-k", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("mine", cx);
            window.press("tab", cx);
            window.input("again", cx);
            window.press("secondary-enter", cx);
        });
        assert!(cx.update(|cx| workspace.read(cx).new_command.is_some()));
        assert_eq!(
            jig_commands::presets::parse(&std::fs::read_to_string(&commands).unwrap())
                .unwrap()
                .len(),
            1
        );
    }

    #[gpui_kit::test]
    fn escape_cancels_the_form(cx: &mut TestAppContext) {
        let (_dir, window, workspace, commands) = open_with_commands(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-k", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("Half typed", cx);
            window.press("escape", cx);
        });
        assert!(cx.update(|cx| workspace.read(cx).new_command.is_none()));
        assert!(!commands.exists());
    }

    #[gpui_kit::test]
    fn the_title_bar_button_opens_the_command_input(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = three_files(cx);
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| window.render_frame(cx));
        let button = vcx
            .debug_bounds("open-command")
            .expect("the button is in the title bar");
        vcx.simulate_click(button.center(), gpui_kit::Modifiers::none());
        vcx.run_until_parked();
        assert!(vcx.update(|_, cx| workspace.read(cx).palette.is_some()));
    }

    #[gpui_kit::test]
    fn a_button_beside_the_selection_opens_the_command_input(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = three_files(cx);
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| window.render_frame(cx));
        assert!(
            vcx.debug_bounds("selection-command").is_none(),
            "nothing selected, no button"
        );

        let ws = workspace.clone();
        vcx.update(|window, cx| {
            ws.update(cx, |this, cx| {
                this.editor().focus(window, cx);
                this.editor()
                    .state()
                    .update(cx, |s, cx| s.set_selected_range(0..7, cx))
            });
            window.render_frame(cx);
        });
        vcx.run_until_parked();
        vcx.update(|window, cx| window.render_frame(cx));
        let button = vcx
            .debug_bounds("selection-command")
            .expect("the button shows beside the selection");
        let line = vcx
            .update(|_, cx| workspace.read(cx).editor().beside_point(0..7, cx))
            .unwrap();
        assert!(button.left() >= line.x, "right of the selected text");

        // Settings can turn it off, and back on.
        vcx.update(|window, cx| {
            crate::settings::update(cx, |s| s.editor.selection_button = false);
            window.render_frame(cx);
        });
        vcx.run_until_parked();
        vcx.update(|window, cx| window.render_frame(cx));
        assert!(
            vcx.debug_bounds("selection-command").is_none(),
            "turned off in Settings"
        );
        vcx.update(|window, cx| {
            crate::settings::update(cx, |s| s.editor.selection_button = true);
            window.render_frame(cx);
        });
        vcx.run_until_parked();
        vcx.update(|window, cx| window.render_frame(cx));
        let button = vcx
            .debug_bounds("selection-command")
            .expect("back once turned on");

        vcx.simulate_click(button.center(), gpui_kit::Modifiers::none());
        vcx.run_until_parked();
        vcx.update(|window, cx| window.render_frame(cx));
        assert!(vcx.update(|_, cx| workspace.read(cx).palette.is_some()));
        assert!(
            vcx.debug_bounds("selection-command").is_none(),
            "hidden while the command input is open"
        );
        assert_eq!(
            vcx.update(|_, cx| workspace.read(cx).editor().selection(cx)),
            0..7,
            "the click kept the selection"
        );
    }

    #[gpui_kit::test]
    fn the_agent_conversation_moves_by_its_header(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = three_files(cx);
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| this.open_test_conversation(window, cx))
        });
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| window.render_frame(cx));
        let header = vcx
            .debug_bounds("agent-chat-header")
            .expect("the conversation is open");
        let before = vcx.update(|_, cx| workspace.read(cx).run.as_ref().unwrap().anchor);

        let (from, by) = (header.center(), gpui_kit::point(px(60.), px(90.)));
        let none = gpui_kit::Modifiers::none();
        vcx.simulate_mouse_down(from, gpui_kit::MouseButton::Left, none);
        for i in 1..=5 {
            let at = from + by * (i as f32 / 5.);
            vcx.simulate_mouse_move(at, Some(gpui_kit::MouseButton::Left), none);
        }
        vcx.simulate_mouse_up(from + by, gpui_kit::MouseButton::Left, none);
        vcx.run_until_parked();

        let after = vcx.update(|_, cx| workspace.read(cx).run.as_ref().unwrap().anchor);
        // Within a pixel: positions are snapped to whole pixels when drawn.
        let near = |a: gpui_kit::Point<gpui_kit::Pixels>, b: gpui_kit::Point<gpui_kit::Pixels>| {
            (a.x - b.x).abs() <= px(1.) && (a.y - b.y).abs() <= px(1.)
        };
        assert!(
            near(after, before + by),
            "it moved with the mouse: {after:?}"
        );
        vcx.update(|window, cx| window.render_frame(cx));
        let moved = vcx.debug_bounds("agent-chat-header").unwrap();
        // The panel may still be settling into place from its slide-in.
        let offset = moved.origin - header.origin;
        assert!(
            (offset.x - by.x).abs() <= px(1.) && (offset.y - by.y).abs() <= px(7.),
            "drawn where it was dropped: {offset:?}"
        );
    }

    #[gpui_kit::test]
    fn clicking_anywhere_in_the_reply_box_focuses_it(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = three_files(cx);
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| this.open_test_conversation(window, cx))
        });
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| window.render_frame(cx));
        let reply = vcx
            .debug_bounds("agent-reply-box")
            .expect("the reply box is shown between turns");
        // In the box's padding, well clear of the text.
        let edge = gpui_kit::point(reply.right() - px(3.), reply.center().y);
        vcx.simulate_click(edge, gpui_kit::Modifiers::none());
        vcx.run_until_parked();
        vcx.update(|window, cx| {
            let this = workspace.read(cx);
            let input = this
                .run
                .as_ref()
                .unwrap()
                .agent
                .as_ref()
                .unwrap()
                .input
                .clone();
            assert!(input.focus_handle(cx).is_focused(window));
        });
    }

    #[gpui_kit::test]
    fn the_agent_conversation_opens_beside_the_code_when_it_fits(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let short = "fn short() {}\n";
        let long = format!("// {}\n", "x".repeat(110));
        std::fs::write(dir.path().join("a.rs"), format!("{short}{long}")).unwrap();
        let (window, workspace) = open(cx, &dir.path().join("a.rs"));

        // Open on `line`, and say where it ended up against that line's end.
        let open_on = |cx: &mut TestAppContext, line: std::ops::Range<usize>| {
            let (ws, selected) = (workspace.clone(), line.clone());
            step(cx, window, move |window, cx| {
                window.render_frame(cx);
                ws.update(cx, |this, cx| {
                    this.editor().select(selected.clone(), cx);
                    this.open_test_conversation(window, cx);
                })
            });
            cx.update(|cx| {
                let this = workspace.read(cx);
                let end = this
                    .editor()
                    .state()
                    .read(cx)
                    .range_to_bounds(&(line.end - 1..line.end - 1))
                    .unwrap();
                (this.run.as_ref().unwrap().anchor, end)
            })
        };

        let (anchor, end) = open_on(cx, 0..short.len());
        assert!(
            anchor.x > end.left(),
            "right of the code: {anchor:?} {end:?}"
        );
        assert!((anchor.y - end.top()).abs() < px(8.), "level with it");

        let (anchor, end) = open_on(cx, short.len()..short.len() + long.len());
        assert!(
            anchor.y >= end.bottom(),
            "no room on the right, so below it: {anchor:?} {end:?}"
        );
    }

    #[gpui_kit::test]
    fn the_agent_conversation_resizes_by_its_corner(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = three_files(cx);
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| this.open_test_conversation(window, cx))
        });
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| window.render_frame(cx));
        let grip = vcx
            .debug_bounds("agent-chat-resize")
            .expect("a corner grip");
        let header = vcx.debug_bounds("agent-chat-header").unwrap();

        let (from, by) = (grip.center(), gpui_kit::point(px(80.), px(60.)));
        let none = gpui_kit::Modifiers::none();
        vcx.simulate_mouse_down(from, gpui_kit::MouseButton::Left, none);
        for i in 1..=5 {
            let at = from + by * (i as f32 / 5.);
            vcx.simulate_mouse_move(at, Some(gpui_kit::MouseButton::Left), none);
        }
        vcx.simulate_mouse_up(from + by, gpui_kit::MouseButton::Left, none);
        vcx.run_until_parked();
        vcx.update(|window, cx| window.render_frame(cx));

        let wider = vcx.debug_bounds("agent-chat-header").unwrap();
        assert_eq!(
            wider.size.width,
            header.size.width + by.x,
            "wider by the drag"
        );
        assert_eq!(
            wider.origin.x, header.origin.x,
            "growing from its left edge"
        );
        let moved = vcx.debug_bounds("agent-chat-resize").unwrap().origin - grip.origin;
        assert!(
            (moved.x - by.x).abs() <= px(1.) && (moved.y - by.y).abs() <= px(7.),
            "the corner followed the mouse: {moved:?}"
        );
    }

    #[gpui_kit::test]
    fn clicks_on_the_form_stay_in_the_form(cx: &mut TestAppContext) {
        let (_dir, window, workspace, _) = open_with_commands(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-k", cx);
        });
        // On the form's title, away from its inputs, over the code. The
        // form is 460 wide, centred, just under the 46px title bar.
        step(cx, window, |window, cx| {
            let title = gpui_kit::point(px(400.), px(74.));
            window.drag(title, title, cx);
        });
        step(cx, window, |window, cx| {
            let this = workspace.read(cx);
            assert!(
                !this.editor().state().focus_handle(cx).is_focused(window),
                "the editor behind didn't take the click"
            );
            window.press("escape", cx);
        });
        assert!(cx.update(|cx| workspace.read(cx).new_command.is_none()));
    }

    #[gpui_kit::test]
    fn escape_closes_the_form_after_clicking_the_code(cx: &mut TestAppContext) {
        let (_dir, window, workspace, _) = open_with_commands(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-k", cx);
        });
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            let editor = ws.read(cx).editor().state().clone();
            editor.update(cx, |editor, cx| editor.focus(window, cx));
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("escape", cx);
        });
        assert!(cx.update(|cx| workspace.read(cx).new_command.is_none()));
    }

    #[gpui_kit::test]
    fn saving_the_commands_file_reloads_presets(cx: &mut TestAppContext) {
        let (_dir, window, workspace, commands) = open_with_commands(cx);
        cx.update(|cx| {
            cx.update_window(window, |_, window, cx| {
                workspace.update(cx, |this, cx| {
                    this.edit_commands(&super::EditCommands, window, cx)
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        assert!(commands.exists(), "created with a header");
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| {
                let end = this.editor().text(cx).len();
                this.editor().apply_edit(
                    end..end,
                    "\n[[jig]]\nname = \"From file\"\nprompt = \"p\"\n",
                    window,
                    cx,
                );
            });
            window.press("secondary-s", cx);
        });
        assert!(preset_names(cx, &workspace).contains(&"From file".to_string()));
    }

    /// Records the user message of each request.
    struct RecordingProvider(std::sync::Mutex<Vec<String>>);

    impl Provider for RecordingProvider {
        fn complete(&self, _: &str, user: &str) -> anyhow::Result<String> {
            self.0.lock().unwrap().push(user.to_string());
            Ok(r#"{"replace": "struct User;", "message": "Added."}"#.into())
        }
    }

    #[gpui_kit::test]
    fn note_reaches_the_model(cx: &mut TestAppContext) {
        let (_dir, window, workspace, commands) = open_with_commands(cx);
        std::fs::create_dir_all(commands.parent().unwrap()).unwrap();
        std::fs::write(
            &commands,
            "[[jig]]\nname = \"Create model\"\nscope = \"cursor\"\nprompt = \"Insert a model.\"\ncomment = \"required\"\n",
        )
        .unwrap();
        let provider = Arc::new(RecordingProvider(Default::default()));
        let provider_for_ws: Arc<dyn Provider> = provider.clone();
        cx.update_window(window, |_, window, cx| {
            workspace.update(cx, |this, cx| {
                this.provider = Ok(provider_for_ws);
                this.reload_presets(window, cx);
            })
        })
        .unwrap();

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-k", cx);
        });
        // Typed and confirmed in one frame, like a fast paste.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("model", cx);
            window.press("enter", cx);
        });
        step(cx, window, |window, cx| {
            window.input("User with an email", cx);
            window.press("enter", cx);
        });

        let sent = provider.0.lock().unwrap().clone();
        assert_eq!(sent.len(), 1);
        assert!(
            sent[0].contains("Instruction: Insert a model.\nNote: User with an email\n"),
            "{}",
            sent[0]
        );
    }

    #[gpui_kit::test]
    fn agents_md_goes_with_every_quick_command(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "Use thiserror for errors.\n").unwrap();
        let path = dir.path().join("src/lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        let provider = Arc::new(RecordingProvider(Default::default()));
        let provider_for_ws: Arc<dyn Provider> = provider.clone();
        cx.update(|cx| workspace.update(cx, |this, _| this.provider = Ok(provider_for_ws)));

        run_preset(cx, window, &workspace, 0..9, "docs");
        let first = provider.0.lock().unwrap()[0].clone();
        assert!(
            first.starts_with("<project_rules>\nUse thiserror for errors.\n</project_rules>"),
            "{first}"
        );

        // Edits to AGENTS.md apply to the next command without a restart.
        step(cx, window, |window, cx| window.press("escape", cx));
        std::fs::write(dir.path().join("AGENTS.md"), "Prefer anyhow.\n").unwrap();
        run_preset(cx, window, &workspace, 0..9, "docs");
        let second = provider.0.lock().unwrap()[1].clone();
        assert!(second.contains("Prefer anyhow."), "{second}");
    }

    fn tree_rows(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
        cx.update(|cx| {
            let tree = workspace.read(cx).tree.as_ref().unwrap().view.read(cx);
            tree.model()
                .rows()
                .iter()
                .map(|row| format!("{}{}", "  ".repeat(row.depth), row.name))
                .collect()
        })
    }

    fn tree_focused(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        ws: &Entity<Workspace>,
    ) -> bool {
        let ws = ws.clone();
        cx.update_window(window, move |_, window, cx| {
            ws.read(cx)
                .tree
                .as_ref()
                .unwrap()
                .view
                .read(cx)
                .is_focused(window)
        })
        .unwrap()
    }

    #[gpui_kit::test]
    fn opening_a_folder_browses_and_opens_files(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/ui")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), ORIGINAL).unwrap();
        std::fs::write(dir.path().join("README.md"), "# hi\n").unwrap();
        let (window, workspace) = open(cx, dir.path());

        assert!(cx.update(|cx| workspace.read(cx).sidebar_open));
        assert!(tree_focused(cx, window, &workspace), "starts in the tree");
        assert_eq!(tree_rows(cx, &workspace), ["src", "README.md"]);

        // Expand src, go to lib.rs, open it.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        });
        assert_eq!(
            tree_rows(cx, &workspace),
            ["src", "  ui", "  lib.rs", "README.md"]
        );
        step(cx, window, |window, cx| {
            window.press("down", cx);
            window.press("right", cx);
            window.press("down", cx);
            window.press("enter", cx);
        });
        assert_eq!(text(cx, &workspace), ORIGINAL);
        step(cx, window, |window, cx| {
            let this = workspace.read(cx);
            assert!(
                this.document()
                    .path
                    .as_ref()
                    .unwrap()
                    .ends_with("src/lib.rs")
            );
            assert!(this.editor().state().focus_handle(cx).is_focused(window));
        });
        assert_eq!(
            tree_rows(cx, &workspace),
            ["src", "  ui", "  lib.rs", "README.md"],
            "an empty folder expands to nothing"
        );
    }

    fn quick_open_rows(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
        cx.update(|cx| {
            let open = workspace.read(cx).quick_open.as_ref().unwrap();
            open.view.read(cx).row_paths()
        })
    }

    #[gpui_kit::test]
    fn go_to_file_finds_and_opens_files(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src/ui")).unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), ORIGINAL).unwrap();
        std::fs::write(dir.path().join("src/ui/button.rs"), "").unwrap();
        std::fs::write(dir.path().join("target/lib.rs"), "").unwrap();
        std::fs::write(dir.path().join("README.md"), "# hi\n").unwrap();
        let (window, workspace) = open(cx, dir.path());

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-p", cx);
        });
        assert_eq!(
            quick_open_rows(cx, &workspace),
            [".gitignore", "README.md", "src/lib.rs", "src/ui/button.rs"],
            "ignored files are left out"
        );

        step(cx, window, |window, cx| window.input("lib", cx));
        assert_eq!(quick_open_rows(cx, &workspace), ["src/lib.rs"]);
        step(cx, window, |window, cx| window.press("enter", cx));
        assert!(cx.update(|cx| workspace.read(cx).quick_open.is_none()));
        assert_eq!(text(cx, &workspace), ORIGINAL);

        // Open a second file; the first is now the most recent other one.
        step(cx, window, |window, cx| {
            window.press("secondary-p", cx);
            window.input("btn", cx);
            window.press("enter", cx);
        });
        step(cx, window, |window, cx| window.press("secondary-p", cx));
        assert_eq!(
            quick_open_rows(cx, &workspace)[0],
            "src/lib.rs",
            "recent files come first, without the current one"
        );
        step(cx, window, |window, cx| window.press("escape", cx));
        step(cx, window, |window, cx| {
            let this = workspace.read(cx);
            assert!(this.quick_open.is_none());
            assert!(this.editor().state().focus_handle(cx).is_focused(window));
            assert!(
                this.document()
                    .path
                    .as_ref()
                    .unwrap()
                    .ends_with("src/ui/button.rs")
            );
        });
    }

    fn find_results(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
        cx.update(|cx| {
            let open = workspace.read(cx).find_in_files.as_ref().unwrap();
            let view = open.view.read(cx);
            assert!(!view.is_searching());
            view.result_lines()
        })
    }

    #[gpui_kit::test]
    fn find_in_files_searches_the_project_and_opens_the_match(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join("target")).unwrap();
        std::fs::write(dir.path().join(".gitignore"), "target/\n").unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn main() {\n    let x = 1;\n    lib::alpha();\n}\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("target/out.rs"), "alpha").unwrap();
        let (window, workspace) = open(cx, dir.path());

        // Into the editor on lib.rs, with `alpha` selected.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            // Typed and chosen before the project's first walk is done.
            window.press("secondary-p", cx);
            window.input("lib", cx);
            window.press("enter", cx);
        });
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(7..12, cx);
        });

        // ⇧⌘F from the editor, which GPUI Kit binds to Replace.
        step(cx, window, |window, cx| {
            window.press("secondary-shift-f", cx)
        });
        assert_eq!(
            cx.update(|cx| workspace
                .read(cx)
                .find_in_files
                .as_ref()
                .unwrap()
                .view
                .read(cx)
                .query(cx)),
            "alpha",
            "the selection is the query"
        );
        assert_eq!(
            find_results(cx, &workspace),
            [
                "src/lib.rs:1: pub fn alpha() {}",
                "src/main.rs:3: lib::alpha();"
            ],
            "ignored files aren't searched"
        );

        step(cx, window, |window, cx| {
            window.press("down", cx);
            window.press("enter", cx);
        });
        step(cx, window, |window, cx| {
            let this = workspace.read(cx);
            assert!(this.find_in_files.is_none());
            assert!(
                this.document()
                    .path
                    .as_ref()
                    .unwrap()
                    .ends_with("src/main.rs")
            );
            let text = this.editor().text(cx);
            assert_eq!(&text[this.editor().selection(cx)], "alpha");
            assert!(this.editor().state().focus_handle(cx).is_focused(window));
        });

        // Reopening starts from the last search; options narrow it.
        step(cx, window, |window, cx| {
            window.press("secondary-shift-f", cx)
        });
        step(cx, window, |window, cx| window.input("let", cx));
        assert_eq!(find_results(cx, &workspace), ["src/main.rs:2: let x = 1;"]);
        step(cx, window, |window, cx| {
            window.press("secondary-a", cx);
            window.input("ALPHA", cx);
            window.press("alt-c", cx);
        });
        assert!(find_results(cx, &workspace).is_empty(), "match case is on");
        step(cx, window, |window, cx| window.press("escape", cx));
        assert!(cx.update(|cx| workspace.read(cx).find_in_files.is_none()));
    }

    #[gpui_kit::test]
    fn find_in_files_keeps_the_file_name_in_view(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let long = format!("let needle = \"{}\";\n", "x".repeat(400));
        std::fs::write(dir.path().join("long_file_name.rs"), long).unwrap();
        let (window, workspace) = open(cx, dir.path());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-f", cx);
        });
        step(cx, window, |window, cx| window.input("needle", cx));
        assert_eq!(find_results(cx, &workspace).len(), 1);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            let row = window.find(("find-in-files-row", 0usize)).bounds();
            let file = window.find(("find-in-files-file", 0usize)).bounds();
            let width = px(crate::find_in_files::WIDTH);
            assert!(row.size.width < width, "the row {row:?} fits the panel");
            assert!(
                file.right() <= row.right() && file.size.width > px(0.),
                "file name {file:?} outside its row {row:?}"
            );
        });
    }

    fn start_new(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        workspace: &Entity<Workspace>,
        kind: crate::file_tree::NewEntry,
    ) {
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            window.render_frame(cx);
            let tree = ws.read(cx).tree.as_ref().unwrap().view.clone();
            tree.update(cx, |tree, cx| tree.start_new(kind, window, cx));
        });
    }

    #[gpui_kit::test]
    fn new_file_and_folder_from_the_sidebar(cx: &mut TestAppContext) {
        use crate::file_tree::NewEntry;
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), ORIGINAL).unwrap();
        let (window, workspace) = open(cx, dir.path());

        // "src" is selected, so the file goes inside it.
        start_new(cx, window, &workspace, NewEntry::File);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("my", cx);
            window.press("space", cx);
            window.input("file.rs", cx);
            window.press("enter", cx);
        });
        let created = dir.path().join("src/my file.rs");
        assert!(created.is_file(), "space types into the name");
        assert_eq!(
            tree_rows(cx, &workspace),
            ["src", "  lib.rs", "  my file.rs"]
        );
        cx.update(|cx| {
            let open = workspace.read(cx).document().path.clone().unwrap();
            assert_eq!(
                open.canonicalize().unwrap(),
                created.canonicalize().unwrap()
            );
        });

        // The new file is selected, so the folder goes next to it.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-e", cx);
        });
        start_new(cx, window, &workspace, NewEntry::Folder);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("ui", cx);
            window.press("enter", cx);
        });
        assert!(dir.path().join("src/ui").is_dir());
        assert!(tree_focused(cx, window, &workspace));
        assert_eq!(tree_rows(cx, &workspace)[1], "  ui", "folders sort first");

        // "ui" is now selected, so this goes inside it. A name that's
        // taken keeps the field open; Escape drops it.
        std::fs::write(dir.path().join("src/ui/taken.rs"), "kept").unwrap();
        start_new(cx, window, &workspace, NewEntry::File);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("taken.rs", cx);
            window.press("enter", cx);
        });
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/ui/taken.rs")).unwrap(),
            "kept"
        );
        let naming = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                let tree = workspace.read(cx).tree.as_ref().unwrap().view.read(cx);
                tree.naming_error()
            })
        };
        assert_eq!(naming(cx), Some(Some("taken.rs already exists.".into())));
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("escape", cx);
        });
        assert_eq!(naming(cx), None, "Escape drops the name field");
        assert!(
            tree_focused(cx, window, &workspace),
            "Escape returns to the tree"
        );
        assert_eq!(
            tree_rows(cx, &workspace),
            ["src", "  ui", "    taken.rs", "  lib.rs", "  my file.rs"]
        );
    }

    #[gpui_kit::test]
    fn renaming_in_the_sidebar_moves_the_open_tab(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let path = dir.path().join("src/lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-e", cx);
        });
        assert!(tree_focused(cx, window, &workspace));
        // F2 selects the name without its extension, so typing keeps it.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("f2", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("main", cx);
            window.press("enter", cx);
        });
        let renamed = dir.path().join("src/main.rs");
        assert!(renamed.is_file() && !path.exists());
        assert_eq!(tree_rows(cx, &workspace), ["src", "  main.rs"]);
        cx.update(|cx| {
            let ws = workspace.read(cx);
            let open = ws.document().path.clone().unwrap();
            assert_eq!(
                open.canonicalize().unwrap(),
                renamed.canonicalize().unwrap()
            );
            assert_eq!(ws.editor().text(cx), ORIGINAL);
        });
    }

    #[gpui_kit::test]
    fn sidebar_toggles_and_reveals_the_open_file(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "").unwrap();
        let path = dir.path().join("src/lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);

        assert!(
            !cx.update(|cx| workspace.read(cx).sidebar_open),
            "a single file opens without the sidebar"
        );
        assert_eq!(
            tree_rows(cx, &workspace),
            ["src", "  lib.rs", "Cargo.toml"],
            "the project is the git root, with the file revealed"
        );

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-b", cx);
        });
        assert!(cx.update(|cx| workspace.read(cx).sidebar_open));
        assert!(
            !tree_focused(cx, window, &workspace),
            "Cmd+B keeps focus in the editor"
        );

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-e", cx);
        });
        assert!(tree_focused(cx, window, &workspace));
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("escape", cx);
        });
        assert!(
            !tree_focused(cx, window, &workspace),
            "Escape returns to the editor"
        );

        // A file added on disk shows up when the tree refreshes.
        std::fs::write(dir.path().join("src/new.rs"), "").unwrap();
        cx.update(|cx| workspace.update(cx, |this, cx| this.refresh_tree(cx)));
        assert!(tree_rows(cx, &workspace).contains(&"  new.rs".to_string()));

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-b", cx);
        });
        assert!(!cx.update(|cx| workspace.read(cx).sidebar_open));
    }

    fn tab_titles(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
        cx.update(|cx| {
            let this = workspace.read(cx);
            this.tabs.iter().map(|tab| tab.document.title()).collect()
        })
    }

    fn active_title(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> String {
        cx.update(|cx| workspace.read(cx).document().title())
    }

    fn open_file(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        workspace: &Entity<Workspace>,
        path: &Path,
    ) {
        let ws = workspace.clone();
        let path = path.to_path_buf();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| this.open_file(&path, window, cx))
        });
    }

    /// `a.rs` open with `b.rs` and `c.rs` next to it on disk.
    fn three_files(
        cx: &mut TestAppContext,
    ) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
        let dir = tempfile::tempdir().unwrap();
        for name in ["a.rs", "b.rs", "c.rs"] {
            std::fs::write(dir.path().join(name), format!("// {name}\n")).unwrap();
        }
        let (window, workspace) = open(cx, &dir.path().join("a.rs"));
        (dir, window, workspace)
    }

    #[gpui_kit::test]
    fn each_tab_keeps_its_own_buffer_and_undo(cx: &mut TestAppContext) {
        let (dir, window, workspace) = three_files(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("x", cx);
        });
        open_file(cx, window, &workspace, &dir.path().join("b.rs"));
        assert_eq!(tab_titles(cx, &workspace), ["a.rs", "b.rs"]);
        assert_eq!(active_title(cx, &workspace), "b.rs");
        assert!(!cx.update(|cx| workspace.read(cx).tab().dirty));
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            assert!(
                workspace
                    .read(cx)
                    .editor()
                    .state()
                    .focus_handle(cx)
                    .is_focused(window),
                "the new tab has the keyboard"
            );
            window.input("y", cx);
        });
        assert_eq!(text(cx, &workspace), "y// b.rs\n");

        // Back to a.rs: its edit is still there and still unsaved.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-1", cx);
        });
        assert_eq!(text(cx, &workspace), "x// a.rs\n");
        assert!(cx.update(|cx| workspace.read(cx).tab().dirty));

        // Undo in a.rs leaves b.rs alone.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-z", cx);
        });
        assert_eq!(text(cx, &workspace), "// a.rs\n");
        assert!(!cx.update(|cx| workspace.read(cx).tab().dirty));
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-]", cx);
        });
        assert_eq!(text(cx, &workspace), "y// b.rs\n");
    }

    #[gpui_kit::test]
    fn opening_an_open_file_switches_to_its_tab(cx: &mut TestAppContext) {
        let (dir, window, workspace) = three_files(cx);
        open_file(cx, window, &workspace, &dir.path().join("b.rs"));
        open_file(cx, window, &workspace, &dir.path().join("c.rs"));
        open_file(cx, window, &workspace, &dir.path().join("a.rs"));
        assert_eq!(tab_titles(cx, &workspace), ["a.rs", "b.rs", "c.rs"]);
        assert_eq!(active_title(cx, &workspace), "a.rs");

        // New tabs open next to the current one.
        std::fs::write(dir.path().join("d.rs"), "").unwrap();
        open_file(cx, window, &workspace, &dir.path().join("d.rs"));
        assert_eq!(tab_titles(cx, &workspace), ["a.rs", "d.rs", "b.rs", "c.rs"]);

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-9", cx);
        });
        assert_eq!(
            active_title(cx, &workspace),
            "c.rs",
            "Cmd+9 is the last tab"
        );
        step(cx, window, |window, cx| window.press("ctrl-tab", cx));
        assert_eq!(active_title(cx, &workspace), "a.rs", "wraps around");
    }

    #[gpui_kit::test]
    fn a_file_from_elsewhere_keeps_the_project(cx: &mut TestAppContext) {
        let (dir, window, workspace) = three_files(cx);
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(elsewhere.path().join("debuggers.toml"), "").unwrap();
        let root = |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).project_root(cx));
        let project = Some(dir.path().canonicalize().unwrap());

        open_file(
            cx,
            window,
            &workspace,
            &elsewhere.path().join("debuggers.toml"),
        );
        assert_eq!(active_title(cx, &workspace), "debuggers.toml");
        assert_eq!(root(cx), project);

        open_file(cx, window, &workspace, &dir.path().join("b.rs"));
        assert_eq!(root(cx), project);
    }

    #[gpui_kit::test]
    fn closing_tabs(cx: &mut TestAppContext) {
        let (dir, window, workspace) = three_files(cx);
        open_file(cx, window, &workspace, &dir.path().join("b.rs"));
        open_file(cx, window, &workspace, &dir.path().join("c.rs"));
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-2", cx);
        });

        // A clean tab closes at once; its right neighbour takes over.
        step(cx, window, |window, cx| window.press("secondary-w", cx));
        assert_eq!(tab_titles(cx, &workspace), ["a.rs", "c.rs"]);
        assert_eq!(active_title(cx, &workspace), "c.rs");

        // An unsaved one asks first.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("z", cx);
        });
        step(cx, window, |window, cx| window.press("secondary-w", cx));
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(tab_titles(cx, &workspace), ["a.rs", "c.rs"]);
        step(cx, window, |window, cx| window.press("secondary-w", cx));
        cx.simulate_prompt_answer("Discard");
        cx.run_until_parked();
        assert_eq!(
            tab_titles(cx, &workspace),
            ["a.rs"],
            "the last tab closed goes left"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("c.rs")).unwrap(),
            "// c.rs\n"
        );

        // Closing the last file leaves an empty Untitled tab.
        step(cx, window, |window, cx| window.press("secondary-w", cx));
        assert_eq!(tab_titles(cx, &workspace), ["Untitled"]);
        assert_eq!(text(cx, &workspace), "");
    }

    #[gpui_kit::test]
    fn closing_other_and_all_tabs(cx: &mut TestAppContext) {
        let (dir, window, workspace) = three_files(cx);
        open_file(cx, window, &workspace, &dir.path().join("b.rs"));
        open_file(cx, window, &workspace, &dir.path().join("c.rs"));
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-2", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("z", cx);
        });
        let close_except = |cx: &mut TestAppContext, keep: Option<usize>| {
            let ws = workspace.clone();
            step(cx, window, move |window, cx| {
                ws.update(cx, |this, cx| this.close_tabs_except(keep, window, cx))
            });
        };

        // Closing the others asks once about the unsaved b.rs.
        close_except(cx, Some(2));
        assert!(cx.has_pending_prompt());
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(tab_titles(cx, &workspace), ["a.rs", "b.rs", "c.rs"]);
        close_except(cx, Some(2));
        cx.simulate_prompt_answer("Discard");
        cx.run_until_parked();
        assert_eq!(tab_titles(cx, &workspace), ["c.rs"]);
        assert_eq!(active_title(cx, &workspace), "c.rs");

        // Closing them all shows the start page.
        open_file(cx, window, &workspace, &dir.path().join("a.rs"));
        close_except(cx, None);
        assert!(!cx.has_pending_prompt());
        assert!(cx.update(|cx| workspace.read(cx).home));
        assert_eq!(tab_titles(cx, &workspace), ["Untitled"]);
    }

    #[gpui_kit::test]
    fn a_blank_tab_is_reused(cx: &mut TestAppContext) {
        let (dir, window, workspace) = three_files(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-n", cx);
        });
        assert_eq!(tab_titles(cx, &workspace), ["a.rs", "Untitled"]);
        open_file(cx, window, &workspace, &dir.path().join("b.rs"));
        assert_eq!(
            tab_titles(cx, &workspace),
            ["a.rs", "b.rs"],
            "the untouched Untitled tab became b.rs"
        );
    }

    #[gpui_kit::test]
    fn switching_tabs_keeps_a_change_under_review(cx: &mut TestAppContext) {
        let (dir, window, workspace) = preview_docs(cx);
        std::fs::write(dir.path().join("b.rs"), "").unwrap();
        open_file(cx, window, &workspace, &dir.path().join("b.rs"));
        assert!(bubble(cx, &workspace).is_none());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-1", cx);
        });
        assert_eq!(text(cx, &workspace), DOCUMENTED);
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(this.editor().state().read(cx).is_editable());
            assert!(this.editor().highlighted_ranges(cx).is_empty());
        });
    }

    #[gpui_kit::test]
    fn same_named_files_show_their_folder(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        for folder in ["app", "core"] {
            std::fs::create_dir_all(dir.path().join(folder)).unwrap();
            std::fs::write(dir.path().join(folder).join("mod.rs"), "").unwrap();
        }
        std::fs::write(dir.path().join("main.rs"), "").unwrap();
        let (window, workspace) = open(cx, &dir.path().join("app/mod.rs"));
        open_file(cx, window, &workspace, &dir.path().join("core/mod.rs"));
        open_file(cx, window, &workspace, &dir.path().join("main.rs"));
        assert_eq!(
            cx.update(|cx| workspace.read(cx).tab_labels()),
            ["mod.rs — app", "mod.rs — core", "main.rs"]
        );
    }

    #[gpui_kit::test]
    fn settings_changes_reach_open_tabs_and_commands(cx: &mut TestAppContext) {
        let (dir, window, workspace) = three_files(cx);
        assert!(preset_names(cx, &workspace).contains(&"Add docs".to_string()));

        cx.update(|cx| {
            crate::settings::update(cx, |s| {
                s.editor.soft_wrap = false;
                s.commands.set_hidden("add docs", true);
            })
        });
        cx.run_until_parked();
        assert!(
            !preset_names(cx, &workspace).contains(&"Add docs".to_string()),
            "hidden commands leave ⌘K"
        );
        // Tabs opened later start with the new settings too.
        open_file(cx, window, &workspace, &dir.path().join("b.rs"));
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(!this.settings.editor.soft_wrap);
            assert_eq!(this.tabs.len(), 2);
        });

        cx.update(|cx| crate::settings::update(cx, |s| s.commands.hidden.clear()));
        cx.run_until_parked();
        assert!(preset_names(cx, &workspace).contains(&"Add docs".to_string()));
    }

    #[gpui_kit::test]
    fn closing_the_window_asks_about_every_unsaved_tab(cx: &mut TestAppContext) {
        let (dir, window, workspace) = three_files(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("x", cx);
        });
        open_file(cx, window, &workspace, &dir.path().join("b.rs"));
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-shift-w", cx);
        });
        assert!(
            cx.has_pending_prompt(),
            "a.rs is unsaved, though not in view"
        );
        cx.simulate_prompt_answer("Cancel");
        cx.run_until_parked();
        assert_eq!(tab_titles(cx, &workspace), ["a.rs", "b.rs"]);
    }

    /// Wait in real time for the agent, letting the window handle what it
    /// sends, until `done` holds.
    fn wait_for_agent(
        cx: &mut TestAppContext,
        workspace: &Entity<Workspace>,
        what: &str,
        done: impl Fn(&Option<Bubble>) -> bool,
    ) -> Option<Bubble> {
        for _ in 0..240 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(500));
            cx.run_until_parked();
            let bubble = bubble(cx, workspace);
            if done(&bubble) {
                return bubble;
            }
        }
        panic!("timed out waiting for {what}: {:?}", bubble(cx, workspace));
    }

    /// Talks to a real OpenCode: `cargo test -p jig-app agent_live -- --ignored`.
    #[gpui_kit::test]
    #[ignore = "needs OpenCode and network access"]
    fn agent_live_edit_is_reviewed_then_written(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n").unwrap();
        let (window, workspace) = open(cx, &path);

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-k", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("tab", cx);
            window.input("Add a one-line doc comment to add. Nothing else.", cx);
            window.press("enter", cx);
        });
        let started = bubble(cx, &workspace);
        assert!(
            matches!(started, Some(Bubble::Running { .. })),
            "{started:?}"
        );

        let preview = wait_for_agent(cx, &workspace, "an edit", |b| {
            matches!(b, Some(Bubble::Preview { .. } | Bubble::Error(_)))
        });
        assert!(
            matches!(preview, Some(Bubble::Preview { .. })),
            "{preview:?}"
        );
        assert!(
            text(cx, &workspace).contains("///"),
            "the edit is shown in the buffer"
        );
        assert!(
            !std::fs::read_to_string(&path).unwrap().contains("///"),
            "nothing is written before the user accepts"
        );

        step(cx, window, |window, cx| window.press("enter", cx));
        let done = wait_for_agent(cx, &workspace, "the agent to finish", |b| {
            matches!(
                b,
                Some(Bubble::AgentDone(_) | Bubble::Error(_) | Bubble::Preview { .. })
            )
        });
        assert!(matches!(done, Some(Bubble::AgentDone(_))), "{done:?}");
        let disk = std::fs::read_to_string(&path).unwrap();
        assert!(disk.contains("///"), "accepted edit is on disk: {disk}");
        assert_eq!(text(cx, &workspace), disk);
        assert!(!cx.update(|cx| workspace.read(cx).tab().dirty));
        eprintln!("agent said: {done:?}");

        // The reply box has the keyboard; a reply continues the session.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.input("Which function did you document? Answer, don't edit.", cx);
            window.press("enter", cx);
        });
        assert!(matches!(
            bubble(cx, &workspace),
            Some(Bubble::Running { .. })
        ));
        let answer = wait_for_agent(cx, &workspace, "the reply", |b| {
            matches!(b, Some(Bubble::AgentDone(_) | Bubble::Error(_)))
        });
        assert!(matches!(answer, Some(Bubble::AgentDone(_))), "{answer:?}");
        let entries = cx.update(|cx| workspace.read(cx).agent_entries());
        assert!(
            matches!(
                entries.as_slice(),
                [
                    ChatEntry::User(_),
                    ChatEntry::Edit { accepted: true, .. },
                    ChatEntry::Agent(_),
                    ChatEntry::User(_),
                    ChatEntry::Agent(_),
                ]
            ),
            "{entries:?}"
        );
        eprintln!("agent answered: {answer:?}");
        crate::agent::stop();
    }

    #[gpui_kit::test]
    fn f12_goes_to_a_definition_in_another_file_then_to_usages(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn main() {\n    alpha();\n}\n",
        )
        .unwrap();
        let (window, workspace) = open(cx, dir.path());
        open_file(cx, window, &workspace, &dir.path().join("src/main.rs"));

        // On the call: to the function in lib.rs.
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(18..18, cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("f12", cx);
        });
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(
                this.document()
                    .path
                    .as_ref()
                    .unwrap()
                    .ends_with("src/lib.rs")
            );
            assert_eq!(this.editor().selection(cx), 7..12);
        });

        // On the declaration: its usages.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("f12", cx);
        });
        let query = cx.update(|cx| {
            let this = workspace.read(cx);
            this.find_in_files
                .as_ref()
                .map(|open| open.view.read(cx).query(cx))
        });
        assert_eq!(query.as_deref(), Some("alpha"));
    }

    /// Type `text` a character at a time, as completion only follows typing.
    fn type_slowly(cx: &mut TestAppContext, window: AnyWindowHandle, text: &str) {
        for c in text.chars() {
            step(cx, window, move |window, cx| {
                window.render_frame(cx);
                window.input(&c.to_string(), cx);
            });
        }
    }

    /// The labels in the completion list, if it's showing.
    fn completion_labels(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<String> {
        cx.update(|cx| {
            let state = workspace.read(cx).editor().state().read(cx);
            let menu = state.completion_menu_state();
            if !menu.open {
                return Vec::new();
            }
            menu.items.iter().map(|item| item.label.clone()).collect()
        })
    }

    #[gpui_kit::test]
    fn words_in_the_file_complete_without_a_language_server(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "counter = 1\n").unwrap();
        let (window, workspace) = open(cx, &path);
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(12..12, cx);
        });

        type_slowly(cx, window, "co");
        assert!(
            completion_labels(cx, &workspace).is_empty(),
            "too short to guess"
        );
        type_slowly(cx, window, "u");
        assert_eq!(completion_labels(cx, &workspace), ["counter"]);

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        });
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            assert_eq!(editor.text(cx), "counter = 1\ncounter");
        });
        assert!(completion_labels(cx, &workspace).is_empty());
    }

    /// The current tab's text, and what's selected in it.
    fn text_and_selection(
        cx: &mut TestAppContext,
        workspace: &Entity<Workspace>,
    ) -> (String, String) {
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            let text = editor.text(cx);
            let selected = text[editor.selection(cx)].to_string();
            (text, selected)
        })
    }

    fn press(cx: &mut TestAppContext, window: AnyWindowHandle, key: &'static str) {
        step(cx, window, move |window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
        });
    }

    #[gpui_kit::test]
    fn a_snippet_is_filled_in_place_by_place(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.rs");
        std::fs::write(&path, "fn main() {\n    \n}\n").unwrap();
        let (window, workspace) = open(cx, &path);
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(16..16, cx);
        });

        type_slowly(cx, window, "fo");
        assert_eq!(completion_labels(cx, &workspace)[..2], ["for", "fori"]);
        press(cx, window, "enter");
        let (text, selected) = text_and_selection(cx, &workspace);
        assert_eq!(
            text,
            "fn main() {\n    for item in items {\n        \n    }\n}\n"
        );
        assert_eq!(selected, "item");

        type_slowly(cx, window, "x");
        press(cx, window, "tab");
        assert_eq!(text_and_selection(cx, &workspace).1, "items");
        press(cx, window, "shift-tab");
        assert_eq!(text_and_selection(cx, &workspace).1, "x");
        press(cx, window, "tab");
        type_slowly(cx, window, "xs");
        // The last Tab lands in the body and ends the snippet.
        press(cx, window, "tab");
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert_eq!(
                this.editor().text(cx),
                "fn main() {\n    for x in xs {\n        \n    }\n}\n"
            );
            assert_eq!(this.editor().selection(cx), 38..38);
            assert!(this.tab().snippet.is_none());
        });
        // Tab indents again.
        press(cx, window, "tab");
        assert_eq!(
            text_and_selection(cx, &workspace).0,
            "fn main() {\n    for x in xs {\n            \n    }\n}\n"
        );
    }

    #[gpui_kit::test]
    fn var_after_an_expression_names_it(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.ts");
        std::fs::write(&path, "getUser(id)\n").unwrap();
        let (window, workspace) = open(cx, &path);
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(11..11, cx);
        });

        type_slowly(cx, window, ".va");
        assert_eq!(completion_labels(cx, &workspace), ["var"]);
        press(cx, window, "enter");
        let (text, selected) = text_and_selection(cx, &workspace);
        assert_eq!(text, "const user = getUser(id);\n");
        assert_eq!(selected, "user");
        // Escape leaves the snippet; Tab then indents.
        press(cx, window, "escape");
        cx.update(|cx| assert!(workspace.read(cx).tab().snippet.is_none()));
    }

    #[gpui_kit::test]
    fn the_language_server_completes_after_a_dot(cx: &mut TestAppContext) {
        use serde_json::json;

        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let path = dir.path().join("src/main.rs");
        std::fs::write(&path, "fn main() {\n    v\n}\n").unwrap();
        let (client, seen) =
            crate::lsp::fake_server(dir.path().to_path_buf(), move |method, _| match method {
                "textDocument/completion" => json!({"isIncomplete": false, "items": [
                    {"label": "pop", "kind": 2},
                    {"label": "push", "kind": 2, "detail": "fn(&mut self, T)"},
                ]}),
                _ => serde_json::Value::Null,
            });
        let server = crate::lsp::server_for("rust").unwrap();
        let root = crate::lsp::root_for(server, &path);
        cx.update(|cx| crate::lsp::register(server.name, root, client, cx));

        let (window, workspace) = open(cx, dir.path());
        open_file(cx, window, &workspace, &path);
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(17..17, cx);
        });

        let wait_for = |cx: &mut TestAppContext, done: &dyn Fn(&mut TestAppContext) -> bool| {
            for _ in 0..200 {
                cx.run_until_parked();
                if done(cx) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        type_slowly(cx, window, ".");
        wait_for(cx, &|cx| completion_labels(cx, &workspace).len() > 2);
        // The server's, then Jig's postfix snippets.
        let labels = completion_labels(cx, &workspace);
        assert_eq!(labels[..3], ["pop", "push", "for"]);
        assert!(labels.contains(&"var".to_string()));
        type_slowly(cx, window, "pu");
        wait_for(cx, &|cx| completion_labels(cx, &workspace).len() == 1);
        assert_eq!(completion_labels(cx, &workspace), ["push"]);

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("enter", cx);
        });
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            assert_eq!(editor.text(cx), "fn main() {\n    v.push\n}\n");
        });

        // Each request saw the text it was made for.
        let messages: Vec<serde_json::Value> = seen.try_iter().collect();
        let mut text = String::new();
        for message in &messages {
            match message["method"].as_str() {
                Some("textDocument/didChange") => {
                    text = message["params"]["contentChanges"][0]["text"]
                        .as_str()
                        .unwrap()
                        .to_string();
                }
                Some("textDocument/completion") => {
                    let offset = crate::lsp::offset(
                        &text,
                        serde_json::from_value(message["params"]["position"].clone()).unwrap(),
                        crate::lsp::Encoding::Utf8,
                    );
                    assert!(
                        text[..offset].ends_with(['.', 'p', 'u']),
                        "{text:?} at {offset}"
                    );
                }
                _ => {}
            }
        }
        assert!(
            messages
                .iter()
                .any(|m| m["params"]["context"]
                    == json!({"triggerKind": 2, "triggerCharacter": "."}))
        );
    }

    #[gpui_kit::test]
    fn the_language_server_answers_definitions_and_references(cx: &mut TestAppContext) {
        use serde_json::json;

        // The fake server runs on real threads.
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        // Not where a guess would go: only the server knows.
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn main() {\n    renamed();\n}\n",
        )
        .unwrap();
        let lib = crate::lsp::file_uri(&dir.path().join("src/lib.rs"));
        let main = crate::lsp::file_uri(&dir.path().join("src/main.rs"));
        let at = |line: u32, start: u32, end: u32| json!({"start": {"line": line, "character": start}, "end": {"line": line, "character": end}});
        let (client, _seen) =
            crate::lsp::fake_server(dir.path().to_path_buf(), move |method, _| match method {
                "textDocument/definition" => json!({"uri": lib, "range": at(0, 7, 12)}),
                "textDocument/references" => json!([
                    {"uri": lib, "range": at(0, 7, 12)},
                    {"uri": main, "range": at(1, 4, 11)},
                ]),
                _ => serde_json::Value::Null,
            });
        let main_path = dir.path().join("src/main.rs");
        let server = crate::lsp::server_for("rust").unwrap();
        let root = crate::lsp::root_for(server, &main_path);
        cx.update(|cx| crate::lsp::register(server.name, root, client, cx));

        let (window, workspace) = open(cx, dir.path());
        open_file(cx, window, &workspace, &main_path);
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(18..18, cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("f12", cx);
        });
        // The server answers on its own thread.
        let wait_for = |cx: &mut TestAppContext, done: &dyn Fn(&gpui_kit::App) -> bool| {
            for _ in 0..200 {
                cx.run_until_parked();
                if cx.update(|cx| done(cx)) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        wait_for(cx, &|cx| {
            workspace
                .read(cx)
                .document()
                .path
                .as_ref()
                .unwrap()
                .ends_with("src/lib.rs")
        });
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(
                this.document()
                    .path
                    .as_ref()
                    .unwrap()
                    .ends_with("src/lib.rs")
            );
            assert_eq!(this.editor().selection(cx), 7..12);
        });

        // The server says this is the declaration: its references.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("f12", cx);
        });
        wait_for(cx, &|cx| workspace.read(cx).find_in_files.is_some());
        assert_eq!(
            find_results(cx, &workspace),
            [
                "src/lib.rs:1: pub fn alpha() {}",
                "src/main.rs:2: renamed();"
            ]
        );
    }

    #[gpui_kit::test]
    fn cmd_click_goes_to_the_definition(cx: &mut TestAppContext) {
        cmd_click_to_definition(cx, 0);
    }

    #[gpui_kit::test]
    fn cmd_click_works_after_scrolling(cx: &mut TestAppContext) {
        // Far enough down that the editor must scroll more than a screen.
        cmd_click_to_definition(cx, 200);
    }

    /// Cmd+hover then Cmd+click a call `padding` lines down main.rs; it
    /// should land on the function in lib.rs.
    fn cmd_click_to_definition(cx: &mut TestAppContext, padding: usize) {
        use serde_json::json;

        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
        let main = format!(
            "fn main() {{\n{}    renamed();\n}}\n",
            "    // filler\n".repeat(padding)
        );
        let call = main.find("renamed").unwrap();
        std::fs::write(dir.path().join("src/main.rs"), &main).unwrap();
        let lib = crate::lsp::file_uri(&dir.path().join("src/lib.rs"));
        let (client, _seen) =
            crate::lsp::fake_server(dir.path().to_path_buf(), move |method, _| match method {
                "textDocument/definition" => json!({"uri": lib, "range": {
                    "start": {"line": 0, "character": 7}, "end": {"line": 0, "character": 12}}}),
                _ => serde_json::Value::Null,
            });
        let main_path = dir.path().join("src/main.rs");
        let server = crate::lsp::server_for("rust").unwrap();
        let root = crate::lsp::root_for(server, &main_path);
        cx.update(|cx| crate::lsp::register(server.name, root, client, cx));

        let (window, workspace) = open(cx, dir.path());
        open_file(cx, window, &workspace, &main_path);
        // Scroll the call into view.
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(call..call, cx);
        });
        step(cx, window, |window, cx| window.render_frame(cx));
        step(cx, window, |window, cx| window.render_frame(cx));
        let point = cx.update(|cx| {
            let state = workspace.read(cx).editor().state().clone();
            let bounds = state
                .read(cx)
                .range_to_bounds(&(call + 1..call + 2))
                .unwrap();
            bounds.center()
        });
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.simulate_mouse_move(point, None, gpui_kit::Modifiers::secondary_key());
        for _ in 0..100 {
            vcx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        vcx.update(|window, cx| window.render_frame(cx));
        vcx.simulate_click(point, gpui_kit::Modifiers::secondary_key());
        for _ in 0..50 {
            vcx.run_until_parked();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(this.find_in_files.is_none(), "searched instead of jumping");
            assert!(
                this.document()
                    .path
                    .as_ref()
                    .unwrap()
                    .ends_with("src/lib.rs"),
                "Cmd+click at {point:?} didn't jump"
            );
            assert_eq!(this.editor().selection(cx), 7..12);
        });
    }

    /// Let the process threads and the UI catch up until `done`.
    fn wait_until(cx: &mut TestAppContext, mut done: impl FnMut(&mut TestAppContext) -> bool) {
        for _ in 0..500 {
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(50));
            cx.run_until_parked();
            if done(cx) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("timed out");
    }

    #[gpui_kit::test]
    fn runs_a_configuration_and_opens_files_from_its_output(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join(".jig")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), ORIGINAL).unwrap();
        let hello = if cfg!(windows) {
            "echo hello %GREETING%& 1>&2 echo src/lib.rs:1:4: here& exit 2"
        } else {
            "echo hello $GREETING; echo 'src/lib.rs:1:4: here' >&2; exit 2"
        };
        std::fs::write(
            dir.path().join(".jig/run.toml"),
            format!(
                r#"
[[run]]
name = "Other"
command = "exit 0"

[[run]]
name = "Hello"
command = "{hello}"
env = {{ GREETING = "there" }}
"#
            ),
        )
        .unwrap();
        let (window, workspace) = open(cx, dir.path());

        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("ctrl-alt-r", cx);
        });
        wait_until(cx, |cx| {
            cx.update(|cx| workspace.read(cx).run_picker_rows(cx))
                .is_some_and(|rows| rows.len() == 3)
        });
        assert_eq!(
            cx.update(|cx| workspace.read(cx).run_picker_rows(cx))
                .unwrap(),
            ["Other", "Hello", "Edit run.toml…"]
        );
        step(cx, window, |window, cx| window.input("hel", cx));
        step(cx, window, |window, cx| window.press("enter", cx));
        wait_until(cx, |cx| {
            cx.update(|cx| workspace.read(cx).run_output())
                .is_some_and(|(_, _, ending)| ending.is_some())
        });
        let (first, lines, ending) = cx.update(|cx| workspace.read(cx).run_output()).unwrap();
        assert_eq!(ending.as_deref(), Some("Process finished with exit code 2"));
        let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
        assert!(texts[0].starts_with("$ echo hello"));
        assert!(texts.contains(&"hello there".to_string()), "{texts:?}");
        assert!(
            texts
                .last()
                .unwrap()
                .starts_with("Process finished with exit code 2 (")
        );

        let link = lines
            .iter()
            .find_map(|line| line.links.first().cloned())
            .expect("the file reference is a link");
        let target = workspace.clone();
        step(cx, window, move |window, cx| {
            target.update(cx, |this, cx| this.open_link(&link, window, cx));
        });
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(
                this.document()
                    .path
                    .as_ref()
                    .unwrap()
                    .ends_with("src/lib.rs")
            );
            assert_eq!(this.editor().selection(cx), 3..3);
        });

        // ⌃R runs the chosen configuration again.
        step(cx, window, |window, cx| window.press("ctrl-r", cx));
        wait_until(cx, |cx| {
            cx.update(|cx| workspace.read(cx).run_output())
                .is_some_and(|(id, _, ending)| id != first && ending.is_some())
        });
    }

    /// Plays `lldb-dap`'s part, as it answered for a real program: stops at
    /// the breakpoint, shows `numbers` and `total`, and finishes on resume.
    /// Records the breakpoint lines it was given.
    fn fake_adapter(
        source: std::path::PathBuf,
        breakpoints: std::sync::Arc<std::sync::Mutex<Vec<u64>>>,
    ) {
        use crate::dap::{Client, frame, read_message, socket_pair};
        use serde_json::json;
        super::debug::FAKE_ADAPTER.with(|fake| {
            *fake.borrow_mut() = Some(Box::new(move |messages| {
                let (ours, theirs) = socket_pair();
                let source = source.clone();
                let breakpoints = breakpoints.clone();
                std::thread::spawn(move || {
                    let mut reader = std::io::BufReader::new(theirs.try_clone().unwrap());
                    let mut out = theirs;
                    let mut send = |message: serde_json::Value| {
                        use std::io::Write as _;
                        out.write_all(&frame(&message)).unwrap();
                    };
                    while let Some(request) = read_message(&mut reader) {
                        let command = request["command"].as_str().unwrap_or_default().to_string();
                        let respond = |body: serde_json::Value| {
                            json!({"type": "response", "request_seq": request["seq"],
                                   "command": command, "success": true, "body": body})
                        };
                        match command.as_str() {
                            "initialize" => {
                                send(respond(json!({})));
                                send(json!({"type": "event", "event": "initialized"}));
                            }
                            "setBreakpoints" => {
                                for bp in request["arguments"]["breakpoints"].as_array().unwrap() {
                                    breakpoints.lock().unwrap().push(bp["line"].as_u64().unwrap());
                                }
                                send(respond(json!({"breakpoints": []})));
                            }
                            "configurationDone" => {
                                send(respond(json!({})));
                                send(json!({"type": "event", "event": "stopped",
                                            "body": {"reason": "breakpoint", "threadId": 1}}));
                            }
                            "stackTrace" => send(respond(json!({"stackFrames": [
                                {"id": 10, "name": "app::main", "line": 3, "column": 5,
                                 "source": {"path": source}},
                                {"id": 11, "name": "std::rt::lang_start", "line": 1, "column": 1},
                            ]}))),
                            "scopes" => send(respond(json!({"scopes": [
                                {"name": "Locals", "variablesReference": 100, "expensive": false},
                                {"name": "Registers", "variablesReference": 200, "expensive": true},
                            ]}))),
                            "variables" => {
                                let variables = match request["arguments"]["variablesReference"].as_i64() {
                                    Some(100) => json!([
                                        {"name": "numbers", "value": "size=3", "type": "Vec<i32>",
                                         "variablesReference": 101},
                                        {"name": "total", "value": "6", "type": "i32",
                                         "variablesReference": 0},
                                    ]),
                                    _ => json!([
                                        {"name": "[0]", "value": "1", "variablesReference": 0},
                                        {"name": "[1]", "value": "2", "variablesReference": 0},
                                    ]),
                                };
                                send(respond(json!({"variables": variables})));
                            }
                            "continue" => {
                                send(respond(json!({"allThreadsContinued": true})));
                                send(json!({"type": "event", "event": "output",
                                            "body": {"category": "stdout", "output": "total 6\n"}}));
                                send(json!({"type": "event", "event": "exited",
                                            "body": {"exitCode": 0}}));
                                send(json!({"type": "event", "event": "terminated"}));
                            }
                            "disconnect" => {
                                send(respond(json!({})));
                                return;
                            }
                            _ => send(respond(json!({}))),
                        }
                    }
                });
                Client::connect(0, ours.try_clone().unwrap(), ours, messages)
            }));
        });
    }

    #[gpui_kit::test]
    fn debugs_to_a_breakpoint_shows_variables_and_resumes(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("src/main.rs");
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join(".jig")).unwrap();
        std::fs::write(
            &source,
            "fn main() {\n    let numbers = vec![1, 2, 3];\n    let total: i32 = numbers.iter().sum();\n    println!(\"total {total}\");\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join(".jig/run.toml"),
            "[[run]]\nname = \"App\"\ncommand = \"./app\"\nprogram = \"app\"\n",
        )
        .unwrap();
        let source = source.canonicalize().unwrap();
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        fake_adapter(source.clone(), sent.clone());
        let (window, workspace) = open(cx, dir.path());
        open_file(cx, window, &workspace, &source);

        // ⌘F8 on the third line.
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            window.render_frame(cx);
            ws.update(cx, |this, cx| {
                this.editor()
                    .state()
                    .update(cx, |s, cx| s.set_selected_range(50..50, cx))
            });
            window.press("secondary-f8", cx);
        });
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert_eq!(this.breakpoint_lines(this.active, cx), [2].into());
        });

        step(cx, window, |window, cx| window.press("ctrl-d", cx));
        wait_until(cx, |cx| {
            cx.update(|cx| workspace.read(cx).run_picker_rows(cx))
                .is_some_and(|rows| rows.first().is_some_and(|row| row == "App"))
        });
        step(cx, window, |window, cx| window.press("enter", cx));
        wait_until(cx, |cx| {
            cx.update(|cx| workspace.read(cx).debug_variable_rows().len() >= 3)
        });
        assert_eq!(
            *sent.lock().unwrap(),
            [3],
            "1-based lines go to the adapter"
        );
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert_eq!(this.debug_phase(), Some(super::debug::Phase::Paused));
            assert_eq!(this.debug_frames(), ["app::main", "std::rt::lang_start"]);
            assert_eq!(this.execution_line(cx), Some((source.clone(), 2)));
            assert_eq!(
                this.debug_variable_rows(),
                [
                    (0, "Locals".into(), "".into()),
                    (1, "numbers".into(), "size=3".into()),
                    (1, "total".into(), "6".into()),
                    (0, "Registers".into(), "".into()),
                ]
            );
        });

        let ws = workspace.clone();
        step(cx, window, move |_, cx| {
            ws.update(cx, |this, cx| this.expand_variable("numbers", cx))
        });
        wait_until(cx, |cx| {
            cx.update(|cx| workspace.read(cx).debug_variable_rows().len() == 6)
        });
        assert_eq!(
            cx.update(|cx| workspace.read(cx).debug_variable_rows())[2],
            (2, "[0]".to_string(), "1".to_string())
        );

        step(cx, window, |window, cx| window.press("f9", cx));
        wait_until(cx, |cx| {
            cx.update(|cx| workspace.read(cx).run_output())
                .is_some_and(|(_, _, ending)| ending.is_some())
        });
        let (_, lines, ending) = cx.update(|cx| workspace.read(cx).run_output()).unwrap();
        assert_eq!(ending.as_deref(), Some("Debugging finished"));
        let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
        assert!(texts.contains(&"total 6".to_string()), "{texts:?}");
        assert!(texts.contains(&"Process finished with exit code 0".to_string()));
        cx.update(|cx| assert_eq!(workspace.read(cx).execution_line(cx), None));
    }

    #[gpui_kit::test]
    fn clicking_the_gutter_toggles_a_breakpoint(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = three_files(cx);
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| {
                let editor = this.editor().clone();
                editor.apply_edit(0..0, "one\ntwo\nthree\nfour\n", window, cx);
            });
        });
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| window.render_frame(cx));
        let (bounds, line_height) = vcx.update(|_, cx| {
            let state = workspace.read(cx).editor().state().read(cx);
            (state.input_bounds(), state.line_height().unwrap())
        });
        // The line numbers start at the editor's left edge.
        let third_line = gpui_kit::point(
            bounds.origin.x + gpui_kit::px(6.),
            bounds.origin.y + line_height * 2.5,
        );
        let lines = |vcx: &mut gpui_kit::VisualTestContext| {
            vcx.update(|_, cx| {
                let this = workspace.read(cx);
                this.breakpoint_lines(this.active, cx)
            })
        };
        let selection = |vcx: &mut gpui_kit::VisualTestContext| {
            vcx.update(|_, cx| workspace.read(cx).editor().selection(cx))
        };
        let before = selection(&mut vcx);
        vcx.simulate_click(third_line, gpui_kit::Modifiers::none());
        vcx.run_until_parked();
        assert_eq!(lines(&mut vcx), [2].into());
        assert_eq!(
            selection(&mut vcx),
            before,
            "the click doesn't move the cursor"
        );
        vcx.update(|window, cx| window.render_frame(cx));
        vcx.simulate_click(third_line, gpui_kit::Modifiers::none());
        vcx.run_until_parked();
        assert!(lines(&mut vcx).is_empty());
    }

    #[gpui_kit::test]
    fn the_title_bar_debug_button_opens_the_picker(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = three_files(cx);
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| window.render_frame(cx));
        let button = vcx
            .debug_bounds("debug-selected")
            .expect("the button is in the title bar");
        vcx.simulate_click(button.center(), gpui_kit::Modifiers::none());
        vcx.run_until_parked();
        assert!(vcx.update(|_, cx| workspace.read(cx).run_picker_rows(cx).is_some()));
    }

    /// Debugs a real Cargo project with the real `lldb-dap`: builds it,
    /// stops at a breakpoint, reads a variable, resumes. Needs Xcode's tools
    /// and Cargo, and macOS may ask once for permission to debug.
    #[gpui_kit::test]
    #[ignore]
    fn debug_live(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        let source = root.join("src/main.rs");
        std::fs::write(
            &source,
            "fn main() {\n    let numbers = vec![1, 2, 3];\n    let total: i32 = numbers.iter().sum();\n    println!(\"total {total}\");\n}\n",
        )
        .unwrap();
        let (window, workspace) = open(cx, &root);
        open_file(cx, window, &workspace, &source);
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            window.render_frame(cx);
            ws.update(cx, |this, cx| {
                this.editor()
                    .state()
                    .update(cx, |s, cx| s.set_selected_range(100..100, cx))
            });
            window.press("secondary-f8", cx);
        });
        step(cx, window, |window, cx| window.press("ctrl-d", cx));
        let wait = |cx: &mut TestAppContext, done: &dyn Fn(&Workspace, &gpui_kit::App) -> bool| {
            for _ in 0..1200 {
                cx.executor()
                    .advance_clock(std::time::Duration::from_millis(50));
                cx.run_until_parked();
                if cx.update(|cx| done(workspace.read(cx), cx)) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let output = cx.update(|cx| workspace.read(cx).run_output());
            panic!("timed out: {output:#?}");
        };
        wait(cx, &|this, cx| {
            this.run_picker_rows(cx)
                .is_some_and(|rows| rows.iter().any(|row| row == "sample"))
        });
        step(cx, window, |window, cx| window.input("sample", cx));
        step(cx, window, |window, cx| window.press("enter", cx));
        wait(cx, &|this, _| {
            this.debug_variable_rows()
                .iter()
                .any(|(_, name, _)| name == "total")
        });
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert_eq!(this.execution_line(cx), Some((source.clone(), 3)));
            let rows = this.debug_variable_rows();
            eprintln!("{rows:#?}");
            assert!(rows.contains(&(1, "total".into(), "6".into())));
            assert!(rows.contains(&(1, "numbers".into(), "size=3".into())));
        });
        // A `println!` line can have more than one breakpoint location, so
        // resume until it ends.
        for _ in 0..5 {
            step(cx, window, |window, cx| window.press("f9", cx));
            wait(cx, &|this, _| {
                this.run_output()
                    .is_some_and(|(_, _, ending)| ending.is_some())
                    || this.debug_phase() == Some(super::debug::Phase::Paused)
            });
            if cx.update(|cx| workspace.read(cx).debug_phase()) == Some(super::debug::Phase::Ended)
            {
                break;
            }
        }
        let (_, lines, _) = cx.update(|cx| workspace.read(cx).run_output()).unwrap();
        let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
        eprintln!("{texts:#?}");
        assert!(texts.contains(&"total 6".to_string()));
    }

    #[gpui_kit::test]
    fn breakpoints_follow_the_debugging_setting(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("app.ts");
        std::fs::write(&path, "const a = 1;\nconst b = 2;\n").unwrap();
        let (window, workspace) = open(cx, &path);
        let lines = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                let this = workspace.read(cx);
                this.breakpoint_lines(this.active, cx)
            })
        };
        let toggle = |cx: &mut TestAppContext| {
            step(cx, window, |window, cx| {
                window.render_frame(cx);
                window.press("secondary-f8", cx);
            })
        };

        // TypeScript's debugger is off to begin with: no breakpoints.
        toggle(cx);
        assert!(lines(cx).is_empty());

        step(cx, window, |_, cx| {
            crate::settings::update(cx, |s| s.debugging.set_enabled("typescript", true))
        });
        toggle(cx);
        assert_eq!(lines(cx), [0].into());

        // Off again hides it, and on again brings it back.
        step(cx, window, |_, cx| {
            crate::settings::update(cx, |s| s.debugging.set_enabled("typescript", false))
        });
        assert!(lines(cx).is_empty());
        step(cx, window, |_, cx| {
            crate::settings::update(cx, |s| s.debugging.set_enabled("typescript", true))
        });
        assert_eq!(lines(cx), [0].into());
    }

    #[gpui_kit::test]
    fn debugging_a_language_that_is_off_starts_nothing(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".jig")).unwrap();
        std::fs::write(dir.path().join("app.ts"), "console.log(1);\n").unwrap();
        std::fs::write(
            dir.path().join(".jig/run.toml"),
            "[[run]]\nname = \"App\"\ncommand = \"node app.ts\"\n",
        )
        .unwrap();
        let (window, workspace) = open(cx, dir.path());
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("ctrl-d", cx);
        });
        wait_until(cx, |cx| {
            cx.update(|cx| workspace.read(cx).run_picker_rows(cx))
                .is_some_and(|rows| rows.first().is_some_and(|row| row == "App"))
        });
        step(cx, window, |window, cx| window.press("enter", cx));
        assert!(cx.update(|cx| workspace.read(cx).run_output()).is_none());
    }

    /// Debugs `source` in a fresh project with a real debugger, found in
    /// `JIG_DEBUGGERS_DIR`: run `command` under ⌃D with a breakpoint on
    /// `line` (0-based), check `variable` shows `value`, resume, and check
    /// the program printed `printed`.
    #[allow(clippy::too_many_arguments)]
    fn debug_project_live(
        cx: &mut TestAppContext,
        debugger: &'static str,
        files: &[(&str, &str)],
        source: &str,
        command: &str,
        line: u32,
        variable: &str,
        value: &str,
        printed: &str,
    ) {
        assert!(
            std::env::var_os("JIG_DEBUGGERS_DIR").is_some(),
            "JIG_DEBUGGERS_DIR: where the debuggers are installed"
        );
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join(".jig")).unwrap();
        for (name, text) in files {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        let source = root.join(source);
        std::fs::write(
            root.join(".jig/run.toml"),
            format!("[[run]]\nname = \"App\"\ncommand = \"{command}\"\n"),
        )
        .unwrap();
        let (window, workspace) = open(cx, &root);
        step(cx, window, move |_, cx| {
            crate::settings::update(cx, |s| s.debugging.set_enabled(debugger, true))
        });
        open_file(cx, window, &workspace, &source);
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            window.render_frame(cx);
            ws.update(cx, |this, cx| {
                let text = this.editor().text(cx);
                let offset: usize = text
                    .split_inclusive('\n')
                    .take(line as usize)
                    .map(str::len)
                    .sum();
                this.editor()
                    .state()
                    .update(cx, |s, cx| s.set_selected_range(offset..offset, cx))
            });
            window.press("secondary-f8", cx);
        });
        step(cx, window, |window, cx| window.press("ctrl-d", cx));
        let wait = |cx: &mut TestAppContext, done: &dyn Fn(&Workspace, &gpui_kit::App) -> bool| {
            for _ in 0..600 {
                cx.executor()
                    .advance_clock(std::time::Duration::from_millis(50));
                cx.run_until_parked();
                if cx.update(|cx| done(workspace.read(cx), cx)) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let output = cx.update(|cx| workspace.read(cx).run_output());
            panic!("timed out: {output:#?}");
        };
        wait(cx, &|this, cx| {
            this.run_picker_rows(cx)
                .is_some_and(|rows| rows.first().is_some_and(|row| row == "App"))
        });
        step(cx, window, |window, cx| window.press("enter", cx));
        let variable = variable.to_string();
        wait(cx, &|this, _| {
            this.debug_variable_rows()
                .iter()
                .any(|(_, name, _)| *name == variable)
        });
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert_eq!(this.execution_line(cx), Some((source.clone(), line)));
            let rows = this.debug_variable_rows();
            eprintln!("{rows:#?}");
            // Some values carry an id that changes, as Java's `int[3]@8`.
            assert!(
                rows.iter().any(|(depth, name, shown)| {
                    *depth == 1 && *name == variable && shown.starts_with(value)
                }),
                "{rows:#?}"
            );
        });
        step(cx, window, |window, cx| window.press("f9", cx));
        wait(cx, &|this, _| {
            this.run_output()
                .is_some_and(|(_, _, ending)| ending.is_some())
        });
        let (_, lines, _) = cx.update(|cx| workspace.read(cx).run_output()).unwrap();
        let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
        eprintln!("{texts:#?}");
        assert!(texts.contains(&printed.to_string()), "{texts:#?}");
    }

    /// TypeScript on Node with js-debug, through its child session. Needs
    /// Node 23.6+. `JIG_TS_COMMAND="npm start"` tries the way package.json
    /// scripts run.
    #[gpui_kit::test]
    #[ignore]
    fn debug_typescript_live(cx: &mut TestAppContext) {
        let command = std::env::var("JIG_TS_COMMAND").unwrap_or_else(|_| "node main.ts".into());
        debug_project_live(
            cx,
            "typescript",
            &[
                (
                    "main.ts",
                    "function total(numbers: number[]): number {\n  return numbers.reduce((a, b) => a + b, 0);\n}\nconst numbers: number[] = [1, 2, 3];\nconst sum: number = total(numbers);\nconsole.log(`total ${sum}`);\n",
                ),
                (
                    "package.json",
                    r#"{"name": "sample", "scripts": {"start": "node main.ts"}}"#,
                ),
            ],
            "main.ts",
            &command,
            1,
            "numbers",
            "(3) [1, 2, 3]",
            "total 6",
        );
    }

    /// Python with debugpy.
    #[gpui_kit::test]
    #[ignore]
    fn debug_python_live(cx: &mut TestAppContext) {
        debug_project_live(
            cx,
            "python",
            &[(
                "main.py",
                "def total(numbers):\n    return sum(numbers)\n\nnumbers = [1, 2, 3]\nresult = total(numbers)\nprint(f\"total {result}\")\n",
            )],
            "main.py",
            "python3 main.py",
            1,
            "numbers",
            "[1, 2, 3]",
            "total 6",
        );
    }

    /// Java with java-debug in jdtls, which finds the main class and builds
    /// the project itself. Needs a JDK, Maven, and `JIG_LIVE_LSP=1` so the
    /// real jdtls runs; `JIG_CACHE_DIR` keeps its index elsewhere.
    #[gpui_kit::test]
    #[ignore]
    fn debug_java_live(cx: &mut TestAppContext) {
        assert!(
            std::env::var_os("JIG_LIVE_LSP").is_some(),
            "JIG_LIVE_LSP=1: Java's debugger lives in jdtls"
        );
        debug_project_live(
            cx,
            "java",
            &[
                (
                    "pom.xml",
                    "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n  <modelVersion>4.0.0</modelVersion>\n  <groupId>sample</groupId>\n  <artifactId>sample</artifactId>\n  <version>1.0</version>\n  <properties>\n    <maven.compiler.release>21</maven.compiler.release>\n  </properties>\n</project>\n",
                ),
                (
                    "src/main/java/sample/Main.java",
                    "package sample;\n\npublic class Main {\n    static int total(int[] numbers) {\n        int sum = 0;\n        for (int n : numbers) sum += n;\n        return sum;\n    }\n\n    public static void main(String[] args) {\n        int[] numbers = {1, 2, 3};\n        System.out.println(\"total \" + total(numbers));\n    }\n}\n",
                ),
            ],
            "src/main/java/sample/Main.java",
            "mvn -q compile exec:java -Dexec.mainClass=sample.Main",
            4,
            "numbers",
            "int[3]",
            "total 6",
        );
    }

    /// Go with Delve, which builds the program itself.
    #[gpui_kit::test]
    #[ignore]
    fn debug_go_live(cx: &mut TestAppContext) {
        debug_project_live(
            cx,
            "go",
            &[
                (
                    "main.go",
                    "package main\n\nimport \"fmt\"\n\nfunc total(numbers []int) int {\n\tsum := 0\n\tfor _, n := range numbers {\n\t\tsum += n\n\t}\n\treturn sum\n}\n\nfunc main() {\n\tnumbers := []int{1, 2, 3}\n\tfmt.Printf(\"total %d\\n\", total(numbers))\n}\n",
                ),
                ("go.mod", "module sample\n\ngo 1.22\n"),
            ],
            "main.go",
            "go run .",
            6,
            "numbers",
            "[]int len: 3, cap: 3, [1,2,3]",
            "total 6",
        );
    }

    /// A repository with `a.rs` committed as three lines, open in a window.
    fn committed_repo(
        cx: &mut TestAppContext,
    ) -> (tempfile::TempDir, AnyWindowHandle, Entity<Workspace>) {
        let dir = tempfile::tempdir().unwrap();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{args:?}: {output:?}");
        };
        git(&["init", "--quiet", "--initial-branch=main"]);
        git(&["config", "user.email", "test@example.com"]);
        git(&["config", "user.name", "Test"]);
        // Windows runners turn line endings into CRLF on checkout.
        git(&["config", "core.autocrlf", "false"]);
        std::fs::write(dir.path().join("a.rs"), "one\ntwo\nthree\n").unwrap();
        git(&["add", "a.rs"]);
        git(&["commit", "--quiet", "-m", "First"]);
        let (window, workspace) = open(cx, &dir.path().join("a.rs"));
        (dir, window, workspace)
    }

    #[gpui_kit::test]
    fn changed_lines_are_marked_and_a_change_can_be_reverted(cx: &mut TestAppContext) {
        use gpui_kit::base::input::LineChangeKind;
        let (_dir, window, workspace) = committed_repo(cx);
        let rows =
            |cx: &mut TestAppContext| workspace.read_with(cx, |this, cx| this.line_change_rows(cx));
        assert_eq!(rows(cx), vec![]);
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| {
                let editor = this.editor().clone();
                editor.apply_edit(4..7, "TWO", window, cx);
                let end = editor.text(cx).len();
                editor.apply_edit(end..end, "four\n", window, cx);
            });
        });
        assert_eq!(
            rows(cx),
            vec![
                (1..2, LineChangeKind::Modified),
                (3..4, LineChangeKind::Added)
            ]
        );

        // Click the bar beside the changed line.
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| window.render_frame(cx));
        let second_line = vcx.update(|_, cx| {
            let state = workspace.read(cx).editor().state().read(cx);
            state.range_to_bounds(&(4..4)).unwrap()
        });
        // The bar sits just left of the text.
        let bar = gpui_kit::point(
            second_line.left() - gpui_kit::px(3.),
            second_line.center().y,
        );
        vcx.simulate_click(bar, gpui_kit::Modifiers::none());
        vcx.run_until_parked();
        assert!(vcx.update(|_, cx| workspace.read(cx).hunk_popup_open()));
        vcx.update(|window, cx| window.render_frame(cx));
        let revert = vcx.debug_bounds("git-revert").expect("the popup shows");
        vcx.simulate_click(revert.center(), gpui_kit::Modifiers::none());
        vcx.run_until_parked();
        assert_eq!(text_of(&mut vcx, &workspace), "one\ntwo\nthree\nfour\n");
        assert!(!vcx.update(|_, cx| workspace.read(cx).hunk_popup_open()));
        assert_eq!(
            vcx.update(|_, cx| workspace.read(cx).line_change_rows(cx)),
            vec![(3..4, LineChangeKind::Added)]
        );
    }

    fn text_of(vcx: &mut gpui_kit::VisualTestContext, workspace: &Entity<Workspace>) -> String {
        vcx.update(|_, cx| workspace.read(cx).editor().text(cx))
    }

    #[gpui_kit::test]
    fn the_git_panel_commits_the_checked_files_and_the_marks_follow(cx: &mut TestAppContext) {
        let (dir, window, workspace) = committed_repo(cx);
        std::fs::write(dir.path().join("b.rs"), "new\n").unwrap();
        std::fs::write(dir.path().join("c.rs"), "also new\n").unwrap();
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| {
                let editor = this.editor().clone();
                editor.apply_edit(0..3, "ONE", window, cx);
                this.save(&super::Save, window, cx);
            });
        });
        assert_eq!(
            workspace.read_with(cx, |this, cx| this.line_change_rows(cx).len()),
            1
        );
        step(cx, window, |window, cx| {
            window.dispatch_action(Box::new(super::ToggleGitPanel), cx)
        });
        let panel = workspace
            .read_with(cx, |this, _| this.git_panel())
            .expect("the panel opens");
        let files = |cx: &mut TestAppContext| {
            panel.read_with(cx, |panel, _| {
                panel
                    .status()
                    .map(|status| status.files.iter().map(|f| f.path.clone()).collect())
                    .unwrap_or_else(Vec::new)
            })
        };
        assert_eq!(files(cx), vec!["a.rs", "b.rs", "c.rs"]);
        // Changes to tracked files start checked, new files don't.
        let checked =
            |cx: &mut TestAppContext| panel.read_with(cx, |panel, _| panel.checked_files());
        assert_eq!(checked(cx), vec!["a.rs"]);
        let p = panel.clone();
        step(cx, window, move |_, cx| {
            p.update(cx, |panel, cx| panel.toggle_file("b.rs", cx))
        });
        assert_eq!(checked(cx), vec!["a.rs", "b.rs"]);
        let p = panel.clone();
        step(cx, window, move |window, cx| {
            p.update(cx, |panel, cx| {
                panel.set_message("Shout", window, cx);
                panel.commit(window, cx);
            })
        });
        assert!(!panel.read_with(cx, |panel, _| panel.is_busy()));
        assert_eq!(files(cx), vec!["c.rs"], "the unchecked file stays out");
        assert_eq!(checked(cx), Vec::<String>::new());
        let log = std::process::Command::new("git")
            .arg("-C")
            .arg(dir.path())
            .args(["log", "--format=%s"])
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&log.stdout), "Shout\nFirst\n");
        // The commit is the new base: nothing differs from it.
        assert_eq!(
            workspace.read_with(cx, |this, cx| this.line_change_rows(cx)),
            vec![]
        );
    }

    #[gpui_kit::test]
    fn the_activity_bar_switches_and_collapses_the_sidebar(cx: &mut TestAppContext) {
        use super::sidebar::SidebarView;
        let (_dir, window, workspace) = committed_repo(cx);
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        let ws = workspace.clone();
        let click = move |vcx: &mut gpui_kit::VisualTestContext, selector: &'static str| {
            // Skip to the end of the sidebar's motion, so nothing moves
            // between finding the button and clicking it.
            vcx.update(|window, cx| {
                ws.update(cx, |this, _| this.sidebar_motion = None);
                window.render_frame(cx)
            });
            let bounds = vcx.debug_bounds(selector).expect(selector);
            vcx.simulate_click(bounds.center(), gpui_kit::Modifiers::none());
            vcx.run_until_parked();
        };
        let shown = |vcx: &mut gpui_kit::VisualTestContext| {
            vcx.update(|_, cx| {
                let this = workspace.read(cx);
                this.sidebar_shown().then_some(this.sidebar_view)
            })
        };
        assert_eq!(shown(&mut vcx), None, "a single file opens without it");
        click(&mut vcx, "activity-files");
        assert_eq!(shown(&mut vcx), Some(SidebarView::Files));
        click(&mut vcx, "activity-git");
        assert_eq!(shown(&mut vcx), Some(SidebarView::Git));
        assert!(vcx.update(|_, cx| workspace.read(cx).git_panel().is_some()));
        click(&mut vcx, "activity-git");
        assert_eq!(
            shown(&mut vcx),
            None,
            "the shown view's button collapses it"
        );

        click(&mut vcx, "branch-switcher");
        assert!(vcx.update(|_, cx| workspace.read(cx).branch_picker().is_some()));
    }

    #[gpui_kit::test]
    fn the_git_view_follows_changes_made_elsewhere(cx: &mut TestAppContext) {
        use super::git::watch::Touched;
        let (dir, window, workspace) = committed_repo(cx);
        step(cx, window, |window, cx| {
            window.dispatch_action(Box::new(super::ToggleGitPanel), cx)
        });
        let panel = workspace
            .read_with(cx, |this, _| this.git_panel())
            .expect("the view opens");
        let files = |cx: &mut TestAppContext| {
            panel.read_with(cx, |panel, _| {
                panel
                    .status()
                    .map(|status| status.files.iter().map(|f| f.path.clone()).collect())
                    .unwrap_or_else(Vec::<String>::new)
            })
        };
        assert_eq!(files(cx), Vec::<String>::new());

        // Another program writes a file; the watcher reports it.
        std::fs::write(dir.path().join("b.rs"), "new\n").unwrap();
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| this.repo_changed(Touched::Files, window, cx))
        });
        assert_eq!(files(cx), vec!["b.rs"]);

        // A commit in a terminal: the list empties and the bars go.
        std::fs::write(dir.path().join("a.rs"), "changed\n").unwrap();
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{args:?}: {output:?}");
        };
        git(&["add", "a.rs", "b.rs"]);
        git(&["commit", "--quiet", "-m", "Elsewhere"]);
        let ws = workspace.clone();
        step(cx, window, move |window, cx| {
            ws.update(cx, |this, cx| {
                this.reload_changed_files(None, window, cx);
                this.repo_changed(Touched::Git, window, cx)
            })
        });
        assert_eq!(files(cx), Vec::<String>::new());
        assert_eq!(
            workspace.read_with(cx, |this, cx| this.line_change_rows(cx)),
            vec![]
        );
    }

    #[gpui_kit::test]
    fn switching_branches_reloads_open_files(cx: &mut TestAppContext) {
        let (dir, window, workspace) = committed_repo(cx);
        let git = |args: &[&str]| {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(output.status.success(), "{args:?}: {output:?}");
        };
        git(&["switch", "--quiet", "--create", "other"]);
        std::fs::write(dir.path().join("a.rs"), "other\n").unwrap();
        git(&["commit", "--quiet", "-am", "Other"]);
        git(&["switch", "--quiet", "main"]);

        step(cx, window, |window, cx| {
            window.dispatch_action(Box::new(super::SwitchBranch), cx)
        });
        let picker = workspace
            .read_with(cx, |this, _| this.branch_picker())
            .expect("the picker opens");
        // New Branch… first, then the branches, most recent first, but both
        // commits are in the same second.
        let mut names = picker.read_with(cx, |picker, _| picker.row_names());
        assert_eq!(names.remove(0), "New Branch…");
        names.sort();
        assert_eq!(names, vec!["main".to_string(), "other".to_string()]);
        cx.simulate_input(window, "oth");
        cx.simulate_keystrokes(window, "enter");
        cx.run_until_parked();
        assert!(workspace.read_with(cx, |this, _| this.branch_picker().is_none()));
        assert_eq!(text(cx, &workspace), "other\n");
        assert_eq!(
            workspace.read_with(cx, |this, cx| this.line_change_rows(cx)),
            vec![]
        );
    }

    #[gpui_kit::test]
    fn new_branch_from_the_picker_and_the_git_menu(cx: &mut TestAppContext) {
        let (dir, window, workspace) = committed_repo(cx);
        let head = || {
            let output = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(["branch", "--show-current"])
                .output()
                .unwrap();
            String::from_utf8(output.stdout).unwrap().trim().to_string()
        };
        let rows = |cx: &mut TestAppContext| {
            let picker = workspace
                .read_with(cx, |this, _| this.branch_picker())
                .unwrap();
            picker.read_with(cx, |picker, _| picker.row_names())
        };

        // The switcher's first row turns it into the name field.
        step(cx, window, |window, cx| {
            window.dispatch_action(Box::new(super::SwitchBranch), cx)
        });
        cx.run_until_parked();
        cx.simulate_keystrokes(window, "enter");
        cx.run_until_parked();
        assert_eq!(rows(cx), Vec::<String>::new());
        cx.simulate_input(window, "main");
        assert_eq!(rows(cx), ["Taken main"], "a taken name isn't offered");
        cx.simulate_keystrokes(window, "escape");
        cx.run_until_parked();

        // Git > New Branch… opens straight to it.
        step(cx, window, |window, cx| {
            window.dispatch_action(Box::new(super::NewBranch), cx)
        });
        cx.run_until_parked();
        cx.simulate_input(window, "my feature");
        assert_eq!(rows(cx), ["Create my-feature"]);
        cx.simulate_keystrokes(window, "enter");
        cx.run_until_parked();
        assert!(workspace.read_with(cx, |this, _| this.branch_picker().is_none()));
        assert_eq!(head(), "my-feature");
    }

    #[gpui_kit::test]
    fn the_git_views_branch_button_asks_for_a_name(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = committed_repo(cx);
        step(cx, window, |window, cx| {
            window.dispatch_action(Box::new(super::ToggleGitPanel), cx)
        });
        // The sidebar slides open on the clock, clipping the panel until
        // it has.
        std::thread::sleep(std::time::Duration::from_millis(250));
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.run_until_parked();
        vcx.update(|window, cx| window.render_frame(cx));
        let button = vcx
            .debug_bounds("git-new-branch")
            .expect("the button is in the Git view");
        vcx.simulate_click(button.center(), gpui_kit::Modifiers::none());
        vcx.run_until_parked();
        let picker = vcx
            .update(|_, cx| workspace.read(cx).branch_picker())
            .expect("the name field opens");
        vcx.simulate_input("second");
        let rows = vcx.update(|_, cx| picker.read(cx).row_names());
        assert_eq!(rows, ["Create second"]);
    }

    #[gpui_kit::test]
    fn rename_symbol_without_a_server_renames_in_the_file(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        let source = "fn main() {\n    let total = 1;\n    use_it(total, subtotal);\n}\n";
        std::fs::write(&path, source).unwrap();
        let (window, workspace) = open(cx, &path);
        let at = source.find("total").unwrap() + 2;
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(at..at, cx);
        });
        step(cx, window, |window, cx| window.press("f2", cx));
        assert!(workspace.read_with(cx, |this, _| this.rename.is_some()));
        // The old name is selected, so typing replaces it.
        cx.simulate_input(window, "sum");
        cx.simulate_keystrokes(window, "enter");
        cx.run_until_parked();
        assert!(workspace.read_with(cx, |this, _| this.rename.is_none()));
        assert_eq!(
            text(cx, &workspace),
            "fn main() {\n    let sum = 1;\n    use_it(sum, subtotal);\n}\n"
        );
        // In the buffer only, until it's saved.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), source);
        assert!(workspace.read_with(cx, |this, _| this.tab().dirty));

        // Esc leaves it be.
        step(cx, window, |window, cx| window.press("f2", cx));
        cx.simulate_input(window, "other");
        cx.simulate_keystrokes(window, "escape");
        cx.run_until_parked();
        assert!(workspace.read_with(cx, |this, _| this.rename.is_none()));
        assert!(text(cx, &workspace).contains("let sum = 1;"));
    }

    #[gpui_kit::test]
    fn rename_symbol_with_a_server_edits_every_file(cx: &mut TestAppContext) {
        use serde_json::json;

        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let lib_source = "pub fn alpha() {}\n";
        std::fs::write(dir.path().join("src/lib.rs"), lib_source).unwrap();
        std::fs::write(
            dir.path().join("src/main.rs"),
            "fn main() {\n    alpha();\n}\n",
        )
        .unwrap();
        let lib = crate::lsp::file_uri(&dir.path().join("src/lib.rs"));
        let main = crate::lsp::file_uri(&dir.path().join("src/main.rs"));
        let at = |line: u32, start: u32, end: u32| json!({"start": {"line": line, "character": start}, "end": {"line": line, "character": end}});
        let (client, _seen) = crate::lsp::fake_server(
            dir.path().to_path_buf(),
            move |method, params| match method {
                "textDocument/rename" => {
                    let name = params["newName"].clone();
                    json!({"changes": {
                        lib.clone(): [{"range": at(0, 7, 12), "newText": name}],
                        main.clone(): [{"range": at(1, 4, 9), "newText": name}],
                    }})
                }
                _ => serde_json::Value::Null,
            },
        );
        let main_path = dir.path().join("src/main.rs");
        let server = crate::lsp::server_for("rust").unwrap();
        let root = crate::lsp::root_for(server, &main_path);
        cx.update(|cx| crate::lsp::register(server.name, root, client, cx));

        let (window, workspace) = open(cx, dir.path());
        open_file(cx, window, &workspace, &main_path);
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(18..18, cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("f2", cx);
        });
        cx.simulate_input(window, "beta");
        cx.simulate_keystrokes(window, "enter");
        for _ in 0..200 {
            cx.run_until_parked();
            if text(cx, &workspace).contains("beta") {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(text(cx, &workspace), "fn main() {\n    beta();\n}\n");
        cx.update(|cx| {
            let this = workspace.read(cx);
            // Still on main.rs, with lib.rs changed in a tab behind it.
            assert!(
                this.document()
                    .path
                    .as_ref()
                    .unwrap()
                    .ends_with("src/main.rs")
            );
            let lib = this
                .tab_for(&dir.path().join("src/lib.rs"))
                .expect("lib.rs opens");
            assert_eq!(this.tabs[lib].editor.text(cx), "pub fn beta() {}\n");
            assert!(this.tabs[lib].dirty);
        });
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/lib.rs")).unwrap(),
            lib_source
        );
    }

    #[gpui_kit::test]
    fn toggle_line_comment_comments_and_uncomments(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.py");
        std::fs::write(&path, "def a():\n    return 1\n").unwrap();
        let (window, workspace) = open(cx, &path);
        let editor = cx.update(|cx| workspace.read(cx).editor().clone());
        step(cx, window, |window, cx| {
            editor.focus(window, cx);
            editor.select(0..0, cx);
        });
        step(cx, window, |window, cx| window.press("secondary-'", cx));
        assert_eq!(text(cx, &workspace), "# def a():\n    return 1\n");
        step(cx, window, |window, cx| {
            window.dispatch_action(Box::new(super::ToggleLineComment), cx)
        });
        assert_eq!(text(cx, &workspace), "def a():\n    return 1\n");
    }

    #[gpui_kit::test]
    fn line_shortcuts_work_in_the_code_editor(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "one\ntwo\nthree").unwrap();
        let (window, workspace) = open(cx, &path);
        let editor = cx.update(|cx| workspace.read(cx).editor().clone());
        step(cx, window, |window, cx| {
            editor.focus(window, cx);
            editor.select(0..0, cx);
        });
        step(cx, window, |window, cx| window.press("alt-down", cx));
        assert_eq!(text(cx, &workspace), "two\none\nthree");
        step(cx, window, |window, cx| {
            window.press("secondary-shift-d", cx)
        });
        assert_eq!(text(cx, &workspace), "two\none\none\nthree");
        step(cx, window, |window, cx| {
            window.press("secondary-shift-backspace", cx)
        });
        assert_eq!(text(cx, &workspace), "two\none\nthree");
        // The cursor is on the line that took the deleted one's place.
        step(cx, window, |window, cx| window.press("alt-up", cx));
        assert_eq!(text(cx, &workspace), "two\nthree\none");
        step(cx, window, |window, cx| window.press("secondary-l", cx));
        assert_eq!(cx.update(|cx| editor.selection(cx)), 4..9);
    }

    #[gpui_kit::test]
    fn go_to_line_moves_the_cursor(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.txt");
        std::fs::write(&path, "one\ntwo\nthree").unwrap();
        let (window, workspace) = open(cx, &path);
        step(cx, window, |window, cx| {
            window.dispatch_action(Box::new(super::GoToLine), cx)
        });
        assert!(cx.update(|cx| workspace.read(cx).go_to_line.is_some()));
        step(cx, window, |window, cx| window.input("3:2", cx));
        step(cx, window, |window, cx| window.press("enter", cx));
        assert!(cx.update(|cx| workspace.read(cx).go_to_line.is_none()));
        let editor = cx.update(|cx| workspace.read(cx).editor().clone());
        assert_eq!(cx.update(|cx| editor.selection(cx)), 9..9);
    }

    /// The find panel's query, or `None` while it's closed.
    fn find_query(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Option<String> {
        cx.update(|cx| {
            workspace
                .read(cx)
                .find_panel
                .as_ref()
                .map(|open| open.view.read(cx).query(cx))
        })
    }

    fn selection(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> std::ops::Range<usize> {
        cx.update(|cx| workspace.read(cx).editor().selection(cx))
    }

    #[gpui_kit::test]
    fn find_starts_from_the_selection_and_enter_goes_to_the_next_match(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "let foo = 1;\nfoo += foo;\n").unwrap();
        let (window, workspace) = open(cx, &path);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            workspace.read(cx).editor().clone().select(4..7, cx);
            window.press("secondary-f", cx);
        });
        assert_eq!(find_query(cx, &workspace).as_deref(), Some("foo"));
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            let this = workspace.read(cx);
            assert!(
                this.find_panel_focused(window, cx),
                "typing goes to the panel"
            );
            let session = this.editor().state().read(cx).search_session();
            assert!(session.is_active(), "the matches are highlighted");
            assert_eq!(session.matcher.len(), 3);
            assert_eq!(
                session.matcher.current(),
                Some(0),
                "starting at the selection"
            );
        });
        assert_eq!(selection(cx, &workspace), 4..7);

        step(cx, window, |window, cx| window.press("enter", cx));
        assert_eq!(
            selection(cx, &workspace),
            13..16,
            "the next match is selected"
        );
        step(cx, window, |window, cx| window.press("enter", cx));
        assert_eq!(selection(cx, &workspace), 20..23);
        step(cx, window, |window, cx| window.press("enter", cx));
        assert_eq!(selection(cx, &workspace), 4..7, "round to the first");
        step(cx, window, |window, cx| window.press("shift-enter", cx));
        assert_eq!(selection(cx, &workspace), 20..23, "and back");

        step(cx, window, |window, cx| window.press("escape", cx));
        assert_eq!(find_query(cx, &workspace), None, "Esc closes it");
        step(cx, window, |window, cx| {
            let this = workspace.read(cx);
            assert!(this.editor().state().focus_handle(cx).is_focused(window));
            assert!(!this.editor().state().read(cx).search_session().is_active());
        });
        assert_eq!(selection(cx, &workspace), 20..23, "at the last match");

        // A selection over two lines isn't a query: the last one is.
        step(cx, window, |window, cx| {
            workspace.read(cx).editor().clone().select(0..16, cx);
            window.press("secondary-f", cx);
        });
        assert_eq!(find_query(cx, &workspace).as_deref(), Some("foo"));
    }

    #[gpui_kit::test]
    fn find_and_replace_replaces_every_match_as_one_step(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "foo bar foo\nfoo\n").unwrap();
        let (window, workspace) = open(cx, &path);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            workspace.read(cx).editor().clone().select(0..3, cx);
            window.press("secondary-alt-f", cx);
        });
        assert_eq!(find_query(cx, &workspace).as_deref(), Some("foo"));
        assert!(cx.update(|cx| {
            workspace
                .read(cx)
                .find_panel
                .as_ref()
                .unwrap()
                .view
                .read(cx)
                .is_replacing()
        }));
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            // The replace field has the keyboard.
            window.input("baz", cx);
        });
        step(cx, window, |window, cx| {
            window.press("secondary-alt-enter", cx)
        });
        assert_eq!(text(cx, &workspace), "baz bar baz\nbaz\n");
        assert!(cx.update(|cx| workspace.read(cx).tab().dirty));

        step(cx, window, |window, cx| window.press("escape", cx));
        step(cx, window, |window, cx| window.press("secondary-z", cx));
        assert_eq!(text(cx, &workspace), "foo bar foo\nfoo\n", "one undo step");
    }

    #[gpui_kit::test]
    fn replace_is_refused_while_a_change_is_under_review(cx: &mut TestAppContext) {
        let (_dir, window, workspace) = preview_docs(cx);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-alt-f", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            // From the replace field to the query.
            window.press("tab", cx);
            window.input("a()", cx);
        });
        step(cx, window, |window, cx| {
            window.press("tab", cx);
            window.input("b()", cx);
        });
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-alt-enter", cx);
            window.press("enter", cx);
        });
        assert_eq!(text(cx, &workspace), DOCUMENTED, "nothing replaced");
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert_eq!(
                this.editor()
                    .state()
                    .read(cx)
                    .search_session()
                    .matcher
                    .len(),
                1,
                "though there was a match"
            );
            assert!(this.previewing(), "and Enter didn't accept the change");
        });

        // Esc closes the panel first, then rejects the change.
        step(cx, window, |window, cx| window.press("escape", cx));
        assert_eq!(find_query(cx, &workspace), None);
        assert!(cx.update(|cx| workspace.read(cx).previewing()));
        step(cx, window, |window, cx| window.press("escape", cx));
        assert_eq!(text(cx, &workspace), ORIGINAL);
    }

    #[gpui_kit::test]
    fn the_palette_closes_the_find_panel(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, ORIGINAL).unwrap();
        let (window, workspace) = open(cx, &path);
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-f", cx);
        });
        assert!(find_query(cx, &workspace).is_some());
        step(cx, window, |window, cx| window.press("secondary-k", cx));
        assert_eq!(find_query(cx, &workspace), None);
        assert!(cx.update(|cx| workspace.read(cx).palette.is_some()));
        // ⌘F leaves the palette alone.
        step(cx, window, |window, cx| window.press("secondary-f", cx));
        assert_eq!(find_query(cx, &workspace), None);
    }

    /// What the editor holds as the current file's problems: each one's
    /// text and message.
    fn problems(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> Vec<(String, String)> {
        cx.update(|cx| {
            let state = workspace.read(cx).editor().state().read(cx);
            let text = state.value().to_string();
            state
                .diagnostics()
                .unwrap()
                .iter()
                .map(|e| (text[e.range.clone()].to_string(), e.message.to_string()))
                .collect()
        })
    }

    fn report(cx: &mut TestAppContext, path: &Path, items: serde_json::Value) {
        let published = crate::lsp::diagnostics::Published {
            key: crate::lsp::diagnostics::path_key(path),
            version: None,
            items: serde_json::from_value(items).unwrap(),
            encoding: crate::lsp::Encoding::Utf16,
        };
        cx.update(|cx| crate::lsp::diagnostics::publish(published, cx));
        cx.run_until_parked();
    }

    #[gpui_kit::test]
    fn problems_show_count_and_can_be_stepped_through(cx: &mut TestAppContext) {
        use serde_json::json;

        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let path = dir.path().join("src/main.rs");
        let text = "fn main() {\n    let x: u32 = \"é\";\n    let y = 1;\n}\n";
        std::fs::write(&path, text).unwrap();
        let other = dir.path().join("src/other.rs");
        std::fs::write(&other, "pub fn f() {}\n").unwrap();
        let (client, _seen) =
            crate::lsp::fake_server(dir.path().to_path_buf(), |_, _| serde_json::Value::Null);
        let server = crate::lsp::server_for("rust").unwrap();
        let root = crate::lsp::root_for(server, &path);
        cx.update(|cx| crate::lsp::register(server.name, root, client, cx));
        let (window, workspace) = open(cx, dir.path());
        open_file(cx, window, &workspace, &path);
        let counts = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                let counts = workspace.read(cx).problem_counts(cx);
                (counts.errors, counts.warnings)
            })
        };
        let hover = |cx: &mut TestAppContext| {
            cx.update(|cx| {
                let hover = workspace.read(cx).tab().problems.hover.clone();
                hover.update(cx, |hover, cx| hover.message(cx))
            })
        };
        let cursor =
            |cx: &mut TestAppContext| cx.update(|cx| workspace.read(cx).editor().cursor(cx));
        assert_eq!(counts(cx), (0, 0));

        // In UTF-16 columns, which `é` doesn't change but bytes would.
        report(
            cx,
            &path,
            json!([
                {
                    "range": {"start": {"line": 2, "character": 8}, "end": {"line": 2, "character": 9}},
                    "severity": 2,
                    "message": "unused variable `y`",
                },
                {
                    "range": {"start": {"line": 1, "character": 17}, "end": {"line": 1, "character": 20}},
                    "severity": 1,
                    "source": "rustc",
                    "message": "mismatched types",
                },
            ]),
        );
        assert_eq!(counts(cx), (1, 1));
        assert_eq!(
            problems(cx, &workspace),
            [
                ("\"é\"".to_string(), "mismatched types".to_string()),
                ("y".to_string(), "unused variable `y`".to_string()),
            ]
        );
        // Clicking the counts in the status bar goes to the first, and
        // shows its message.
        let error = text.find('"').unwrap();
        let warning = text.find("y =").unwrap();
        {
            let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
            vcx.update(|window, cx| window.render_frame(cx));
            let counts = vcx
                .debug_bounds("problem-counts")
                .expect("the counts are in the status bar");
            vcx.simulate_click(counts.center(), gpui_kit::Modifiers::none());
            vcx.run_until_parked();
            vcx.update(|window, cx| window.render_frame(cx));
            assert!(vcx.debug_bounds("problem").is_some(), "the message shows");
        }
        assert_eq!(cursor(cx), error);

        // ⌥F8 goes from problem to problem, round the end; ⇧⌥F8 back.
        assert_eq!(hover(cx).as_deref(), Some("mismatched types"));
        press(cx, window, "alt-f8");
        assert_eq!(cursor(cx), warning);
        press(cx, window, "alt-f8");
        assert_eq!(cursor(cx), error);
        press(cx, window, "alt-shift-f8");
        assert_eq!(cursor(cx), warning);
        assert_eq!(hover(cx).as_deref(), Some("unused variable `y`"));
        press(cx, window, "escape");
        assert_eq!(hover(cx), None);

        // They move with edits until the server reports again.
        step(cx, window, |window, cx| {
            window.render_frame(cx);
            window.press("secondary-up", cx);
        });
        type_slowly(cx, window, "//\n");
        assert_eq!(
            problems(cx, &workspace),
            [
                ("\"é\"".to_string(), "mismatched types".to_string()),
                ("y".to_string(), "unused variable `y`".to_string()),
            ]
        );
        assert_eq!(counts(cx), (1, 1));
        report(cx, &path, json!([]));
        assert_eq!(counts(cx), (0, 0));
        assert!(problems(cx, &workspace).is_empty());

        // A file's problems wait for it to open.
        report(
            cx,
            &other,
            json!([{
                "range": {"start": {"line": 0, "character": 7}, "end": {"line": 0, "character": 8}},
                "severity": 1,
                "message": "expected `;`",
            }]),
        );
        open_file(cx, window, &workspace, &other);
        assert_eq!(counts(cx), (1, 0));
        assert_eq!(
            problems(cx, &workspace),
            [("f".to_string(), "expected `;`".to_string())]
        );
    }

    #[gpui_kit::test]
    fn fix_sends_the_problems_on_the_line(cx: &mut TestAppContext) {
        use serde_json::json;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.rs");
        let text = "fn main() {\n    let x: u32 = \"s\";\n}\n";
        std::fs::write(&path, text).unwrap();
        let (window, workspace) = open(cx, &path);
        let provider = Arc::new(RecordingProvider(Default::default()));
        let provider_for_ws: Arc<dyn Provider> = provider.clone();
        cx.update(|cx| workspace.update(cx, |this, _| this.provider = Ok(provider_for_ws)));
        report(
            cx,
            &path,
            json!([{
                "range": {"start": {"line": 1, "character": 17}, "end": {"line": 1, "character": 20}},
                "severity": 1,
                "source": "rustc",
                "code": "E0308",
                "message": "mismatched types",
            }]),
        );

        // The cursor on the line, nothing selected.
        let on_line = text.find("let").unwrap();
        run_preset(cx, window, &workspace, on_line..on_line, "fix");
        // Another jig doesn't send them.
        press(cx, window, "escape");
        run_preset(cx, window, &workspace, on_line..on_line, "simplify");

        let sent = provider.0.lock().unwrap().clone();
        assert_eq!(sent.len(), 2);
        assert!(
            sent[0].contains(
                "<diagnostics>\nline 2, error: mismatched types (rustc E0308)\n</diagnostics>\n"
            ),
            "{}",
            sent[0]
        );
        assert!(!sent[1].contains("<diagnostics>"), "{}", sent[1]);
    }

    /// A Rust file at `text` in a project whose server is `answer`.
    fn with_fake_server(
        cx: &mut TestAppContext,
        text: &str,
        answer: impl Fn(&str, &serde_json::Value) -> serde_json::Value + Send + 'static,
    ) -> (
        AnyWindowHandle,
        Entity<Workspace>,
        std::path::PathBuf,
        tempfile::TempDir,
    ) {
        cx.executor().allow_parking();
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        let path = dir.path().join("src/main.rs");
        std::fs::write(&path, text).unwrap();
        let (client, _seen) = crate::lsp::fake_server(dir.path().to_path_buf(), answer);
        let server = crate::lsp::server_for("rust").unwrap();
        let root = crate::lsp::root_for(server, &path);
        cx.update(|cx| crate::lsp::register(server.name, root, client, cx));
        let (window, workspace) = open(cx, dir.path());
        open_file(cx, window, &workspace, &path);
        (window, workspace, path, dir)
    }

    type HoverShown = Option<(Option<String>, Vec<String>)>;

    fn hover_shown(cx: &mut TestAppContext, workspace: &Entity<Workspace>) -> HoverShown {
        cx.update(|cx| {
            let hover = workspace.read(cx).tab().hover.clone();
            hover.read(cx).hover_text(cx)
        })
    }

    #[gpui_kit::test]
    fn hover_shows_what_the_server_says_with_the_problem_there(cx: &mut TestAppContext) {
        use serde_json::json;

        let text = "fn main() {\n    helper();\n}\n";
        let (window, workspace, path, _dir) = with_fake_server(
            cx,
            text,
            |method, _| match method {
                "textDocument/hover" => json!({
                    "contents": {"kind": "markdown", "value": "```rust\nfn helper()\n```\n\nHelps."},
                    "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}},
                }),
                _ => serde_json::Value::Null,
            },
        );
        let name = text.find("helper").unwrap();
        report(
            cx,
            &path,
            json!([{
                "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 10}},
                "severity": 1,
                "message": "cannot find function `helper`",
            }]),
        );
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(name + 2..name + 2, cx);
        });

        // The keys: at the cursor, with the problem above it.
        press(cx, window, "secondary-i");
        wait_until(cx, |cx| hover_shown(cx, &workspace).is_some());
        assert_eq!(
            hover_shown(cx, &workspace),
            Some((
                Some("```rust\nfn helper()\n```\n\nHelps.".to_string()),
                vec!["cannot find function `helper`".to_string()]
            ))
        );
        {
            let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
            vcx.update(|window, cx| window.render_frame(cx));
            assert!(vcx.debug_bounds("code-hover").is_some(), "the panel shows");
        }
        // Moving the cursor puts it away; so does Esc.
        press(cx, window, "right");
        assert_eq!(hover_shown(cx, &workspace), None);
        press(cx, window, "secondary-i");
        wait_until(cx, |cx| hover_shown(cx, &workspace).is_some());
        press(cx, window, "escape");
        assert_eq!(hover_shown(cx, &workspace), None);

        // The pointer: asked once it has rested on the name.
        let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
        vcx.update(|window, cx| window.render_frame(cx));
        let at = vcx.update(|_, cx| {
            let state = workspace.read(cx).editor().state().read(cx);
            state
                .range_to_bounds(&(name + 1..name + 2))
                .unwrap()
                .center()
        });
        vcx.simulate_mouse_move(at, None, gpui_kit::Modifiers::none());
        vcx.run_until_parked();
        assert_eq!(hover_shown(&mut vcx, &workspace), None, "not at once");
        vcx.executor()
            .advance_clock(std::time::Duration::from_millis(600));
        wait_until(&mut vcx, |cx| hover_shown(cx, &workspace).is_some());
        let shown = hover_shown(&mut vcx, &workspace).expect("after resting");
        assert!(shown.0.unwrap().contains("Helps."));
        vcx.update(|window, cx| window.render_frame(cx));
        assert!(vcx.debug_bounds("code-hover").is_some());
        assert!(
            vcx.debug_bounds("problem").is_none(),
            "the problem shows in the hover, not on its own"
        );

        // The palette covers it.
        vcx.update(|window, cx| {
            window.press("secondary-k", cx);
            window.render_frame(cx);
        });
        vcx.run_until_parked();
        vcx.update(|window, cx| window.render_frame(cx));
        assert!(vcx.debug_bounds("code-hover").is_none());
        vcx.update(|window, cx| window.press("escape", cx));
        vcx.run_until_parked();

        // Typing puts it away.
        vcx.update(|window, cx| {
            window.render_frame(cx);
            window.input("x", cx);
        });
        vcx.run_until_parked();
        assert_eq!(hover_shown(&mut vcx, &workspace), None);
    }

    #[gpui_kit::test]
    fn hover_without_a_server_shows_nothing(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "helper here\n").unwrap();
        let (window, workspace) = open(cx, &path);
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(2..2, cx);
        });
        press(cx, window, "secondary-i");
        assert_eq!(hover_shown(cx, &workspace), None);
        type_slowly(cx, window, "(");
        cx.update(|cx| {
            assert!(
                workspace
                    .read(cx)
                    .tab()
                    .hover
                    .read(cx)
                    .signature()
                    .is_none()
            )
        });
    }

    #[gpui_kit::test]
    fn signature_help_follows_the_call(cx: &mut TestAppContext) {
        use serde_json::json;

        let text = "fn main() {\n    \n}\nfn f(a: u32, b: &str) {}\n";
        let (window, workspace, _path, _dir) = with_fake_server(cx, text, |method, params| {
            match method {
                "textDocument/signatureHelp" => {
                    // The second parameter once past the comma, as a server
                    // works it out from the text.
                    let past_comma = params["position"]["character"].as_u64() > Some(7);
                    json!({
                        "signatures": [{
                            "label": "fn f(a: u32, b: &str)",
                            "parameters": [{"label": "a: u32"}, {"label": "b: &str"}],
                        }],
                        "activeSignature": 0,
                        "activeParameter": if past_comma { 1 } else { 0 },
                    })
                }
                _ => serde_json::Value::Null,
            }
        });
        let signature = |cx: &mut TestAppContext| {
            cx.update(|cx| workspace.read(cx).tab().hover.read(cx).signature())
        };
        let active = |cx: &mut TestAppContext| signature(cx).and_then(|(_, active)| active);
        let inside = text.find("\n}").unwrap();
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            editor.select(inside..inside, cx);
        });

        type_slowly(cx, window, "f(");
        wait_until(cx, |cx| signature(cx).is_some());
        assert_eq!(
            signature(cx),
            Some((
                "fn f(a: u32, b: &str)".to_string(),
                Some("a: u32".to_string())
            ))
        );
        {
            let mut vcx = gpui_kit::VisualTestContext::from_window(window, cx);
            vcx.update(|window, cx| window.render_frame(cx));
            assert!(vcx.debug_bounds("signature-help").is_some());
        }
        type_slowly(cx, window, "1,");
        wait_until(cx, |cx| active(cx).as_deref() == Some("b: &str"));
        assert_eq!(active(cx).as_deref(), Some("b: &str"));

        // Leaving the call puts it away.
        press(cx, window, "home");
        assert_eq!(signature(cx), None);

        // So does Esc, and `)`.
        cx.update(|cx| {
            let editor = workspace.read(cx).editor().clone();
            let end = editor.text(cx).find("1,").unwrap() + 2;
            editor.select(end..end, cx);
        });
        type_slowly(cx, window, ",");
        wait_until(cx, |cx| signature(cx).is_some());
        press(cx, window, "escape");
        assert_eq!(signature(cx), None);
        type_slowly(cx, window, ",");
        wait_until(cx, |cx| signature(cx).is_some());
        type_slowly(cx, window, ")");
        assert_eq!(signature(cx), None);
    }

    /// A Rust file under `dir` served by a fake server answering formatting
    /// requests with `answer`, open in a window once the server has said it
    /// formats.
    fn open_with_formatting_server(
        cx: &mut TestAppContext,
        dir: &Path,
        text: &str,
        answer: impl Fn(&str, &serde_json::Value) -> serde_json::Value + Send + 'static,
    ) -> (AnyWindowHandle, Entity<Workspace>, std::path::PathBuf) {
        cx.executor().allow_parking();
        // Only the server formats, never what happens to be installed.
        crate::formatters::use_for_tests("");
        // As the workspace has it, so the server is found for the file.
        let dir = &dir.canonicalize().unwrap();
        let path = dir.join("main.rs");
        std::fs::write(&path, text).unwrap();
        let (client, _seen) = crate::lsp::fake_server(dir.to_path_buf(), answer);
        let server = crate::lsp::server_for("rust").unwrap();
        let root = crate::lsp::root_for(server, &path);
        cx.update(|cx| crate::lsp::register(server.name, root, client.clone(), cx));
        for _ in 0..500 {
            if client.formatting().document {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(client.formatting().document, "the server started");
        let (window, workspace) = open(cx, &path);
        (window, workspace, path)
    }

    #[gpui_kit::test]
    fn format_document_makes_the_servers_edits_as_one_undo_step(cx: &mut TestAppContext) {
        use serde_json::json;

        let dir = tempfile::tempdir().unwrap();
        let at = |line: u32, start: u32, end: u32| json!({"start": {"line": line, "character": start}, "end": {"line": line, "character": end}});
        let original = "fn a(){\nlet x=1;\n}\n";
        let (window, workspace, _) =
            open_with_formatting_server(cx, dir.path(), original, move |method, params| {
                match method {
                    // Last first, as servers often send them.
                    "textDocument/formatting" => {
                        assert_eq!(params["options"]["insertSpaces"], true);
                        json!([
                            {"range": at(1, 5, 6), "newText": " = "},
                            {"range": at(1, 0, 0), "newText": "    "},
                            {"range": at(0, 6, 6), "newText": " "},
                        ])
                    }
                    // Only the selected line, which it was told.
                    "textDocument/rangeFormatting" => {
                        assert_eq!(params["range"], at(1, 0, 8));
                        json!([{"range": at(1, 0, 0), "newText": "\t"}])
                    }
                    _ => serde_json::Value::Null,
                }
            });
        let editor = cx.update(|cx| workspace.read(cx).editor().clone());
        cx.update(|cx| editor.select(8..16, cx));
        press(cx, window, "alt-shift-f");
        wait_until(cx, |cx| {
            cx.update(|cx| editor.text(cx)) == "fn a(){\n\tlet x=1;\n}\n"
        });
        step(cx, window, |window, cx| editor.undo(window, cx));
        cx.update(|cx| editor.select(12..12, cx));
        press(cx, window, "alt-shift-f");
        let formatted = "fn a() {\n    let x = 1;\n}\n";
        wait_until(cx, |cx| cx.update(|cx| editor.text(cx)) == formatted);
        // Still on the `x`.
        let x = formatted.find('x').unwrap();
        assert_eq!(cx.update(|cx| editor.selection(cx)), x..x);
        step(cx, window, |window, cx| editor.undo(window, cx));
        assert_eq!(cx.update(|cx| editor.text(cx)), original, "one undo step");
    }

    #[gpui_kit::test]
    fn format_document_leaves_a_change_under_review_alone(cx: &mut TestAppContext) {
        use serde_json::json;

        let dir = tempfile::tempdir().unwrap();
        let (window, workspace, _) = open_with_formatting_server(
            cx,
            dir.path(),
            "a\nb\n",
            |method, _| match method {
                "textDocument/formatting" => {
                    json!([{"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "newText": "x"}])
                }
                _ => serde_json::Value::Null,
            },
        );
        use_provider(
            cx,
            &workspace,
            Ok(r#"{"replace": "B", "message": "Capitalised."}"#),
        );
        run_preset(cx, window, &workspace, 2..3, "Simplify");
        assert!(cx.update(|cx| workspace.read(cx).previewing()));
        press(cx, window, "alt-shift-f");
        std::thread::sleep(std::time::Duration::from_millis(50));
        cx.run_until_parked();
        let text = cx.update(|cx| workspace.read(cx).editor().text(cx));
        assert_eq!(text, "a\nB\n", "not formatted");
    }

    #[gpui_kit::test]
    fn saving_gives_up_on_a_slow_formatter(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (window, workspace, path) =
            open_with_formatting_server(cx, dir.path(), "fn a() {}\n", |method, _| {
                if method == "textDocument/formatting" {
                    std::thread::sleep(std::time::Duration::from_secs(5));
                }
                serde_json::Value::Null
            });
        cx.update(|cx| crate::settings::update(cx, |s| s.formatting.set_on_save("rust", true)));
        let editor = cx.update(|cx| workspace.read(cx).editor().clone());
        step(cx, window, |window, cx| {
            editor.apply_edit(0..0, "// hi\n", window, cx);
        });
        press(cx, window, "secondary-s");
        // Waiting on the formatter, not yet saved.
        assert!(cx.update(|cx| workspace.read(cx).pending_save.is_some()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "fn a() {}\n");

        cx.executor().advance_clock(super::format::SAVE_TIMEOUT);
        cx.run_until_parked();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "// hi\nfn a() {}\n"
        );
        cx.update(|cx| {
            let this = workspace.read(cx);
            assert!(!this.tab().dirty);
            assert!(this.pending_save.is_none());
            let note = this.status_note.as_ref().unwrap();
            assert_eq!(
                note.message.as_ref(),
                "Saved without formatting. The formatter took more than 2 s."
            );
        });
    }

    #[gpui_kit::test]
    fn trims_and_ends_with_a_newline_on_save_when_asked(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "a  \nb\t").unwrap();
        let (window, workspace) = open(cx, &path);
        press(cx, window, "secondary-s");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a  \nb\t", "off");
        cx.update(|cx| {
            crate::settings::update(cx, |s| {
                s.formatting.trim_trailing_whitespace = true;
                s.formatting.final_newline = true;
            })
        });
        press(cx, window, "secondary-s");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "a\nb\n");
        let text = cx.update(|cx| workspace.read(cx).editor().text(cx));
        assert_eq!(text, "a\nb\n", "the buffer too");
    }

    /// With no language server, the formatter for the language, fed the
    /// buffer on stdin: on save, and with Format Document.
    #[cfg(unix)]
    #[gpui_kit::test]
    fn without_a_server_the_formatter_formats(cx: &mut TestAppContext) {
        cx.executor().allow_parking();
        crate::formatters::use_for_tests(
            r#"
[[formatter]]
key = "missing"
name = "Missing"
languages = ["rust"]
command = ["jig-no-such-formatter"]

[[formatter]]
key = "upper"
name = "Upper"
languages = ["rust"]
command = ["/bin/sh", "-c", "tr a-z A-Z"]
"#,
        );
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("main.rs");
        std::fs::write(&path, "let x = 1;  \nlet y = 2;").unwrap();
        let (window, workspace) = open(cx, &path);
        cx.update(|cx| {
            crate::settings::update(cx, |s| {
                s.formatting.set_on_save("rust", true);
                s.formatting.trim_trailing_whitespace = true;
                s.formatting.final_newline = true;
            })
        });
        press(cx, window, "secondary-s");
        wait_until(cx, |_| {
            std::fs::read_to_string(&path).unwrap() == "LET X = 1;\nLET Y = 2;\n"
        });

        let editor = cx.update(|cx| workspace.read(cx).editor().clone());
        step(cx, window, |window, cx| {
            editor.apply_edit(0..0, "fn b() {}\n", window, cx);
            editor.select(3..3, cx);
        });
        press(cx, window, "alt-shift-f");
        wait_until(cx, |cx| cx.update(|cx| editor.text(cx)).starts_with("FN B"));
        assert_eq!(
            cx.update(|cx| editor.selection(cx)),
            3..3,
            "the cursor stays"
        );
    }
}
