//! The one window: open files in tabs, with the project's files in a
//! sidebar.

mod agent;
mod commands;
mod completions;
mod definitions;
mod find;
mod go_to_file;
mod home;
mod lsp;
mod preferences;
mod run;
mod sidebar;
mod snippets;
mod status_bar;
mod tabs;
mod user_commands;

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use gpui_kit::component::input::{Editor, GoToDefinition, Replace};
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
        FindInFiles,
        NewFile,
        CloseTab,
        NextTab,
        PreviousTab,
        RunSelected,
        ChooseRunConfiguration,
        StopRun,
        ToggleRunPanel,
        EditRunConfigurations
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
    last_find: find::LastFind,
    /// The project's files as last walked, for Go to File to show at once.
    file_index: Option<go_to_file::FileIndex>,
    /// Files shown in this window, most recent first, for Go to File.
    recent_files: Vec<PathBuf>,
    new_command: Option<user_commands::OpenForm>,
    /// The user's commands file, `~/.config/jig/commands.toml`.
    commands_path: Option<PathBuf>,
    /// The configured model, or why it couldn't be set up.
    provider: Result<Arc<dyn Provider>, String>,
    /// The AI providers file, `~/.config/jig/config.toml`.
    config_path: Option<PathBuf>,
    /// The settings as last applied, to tell what a change touched.
    settings: Settings,
    run: Option<CommandRun>,
    next_run_id: u64,
    /// Run configurations, and the one running.
    runs: run::RunState,
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
        let (presets, presets_error) = match presets::load(presets::user_commands_path().as_deref())
        {
            Ok(presets) => (presets, None),
            Err(error) => (presets::defaults(), Some(error)),
        };
        let presets = preferences::visible_presets(presets, &settings);
        let mut this = Self {
            tabs: Vec::new(),
            active: 0,
            tab_scroll: ScrollHandle::new(),
            tree: None,
            sidebar_open: false,
            sidebar_width: px(240.),
            resizing_sidebar: false,
            home: path.is_none() || folder.is_some(),
            home_focus: cx.focus_handle(),
            presets: Rc::new(presets),
            palette: None,
            quick_open: None,
            find_in_files: None,
            last_find: Default::default(),
            file_index: None,
            recent_files: Vec::new(),
            new_command: None,
            commands_path: presets::user_commands_path(),
            provider: commands::load_provider(settings.ai.provider.as_deref()),
            config_path: jig_ai::Config::user_path(),
            settings,
            run: None,
            next_run_id: 0,
            runs: Default::default(),
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
        // A running configuration would outlive Jig otherwise.
        cx.on_app_quit(|this, _| {
            this.stop_process();
            async {}
        })
        .detach();
        // Pick up files added or removed outside Jig.
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.refresh_tree(cx);
            }
        })
        .detach();
        if let Some(error) = presets_error {
            this.show_error(
                &format!("Using the built-in commands. {error:#}"),
                window,
                cx,
            );
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
    /// the user agrees to discard them. Checks the tab `only`, or every tab.
    fn when_discard_ok(
        &mut self,
        only: Option<EntityId>,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) {
        let dirty: Vec<String> = self
            .tabs
            .iter()
            .filter(|tab| tab.dirty && only.is_none_or(|id| tab.id() == id))
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
            this.runs = Default::default();
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
            Some(path) => self.save_to(&path, window, cx),
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
            this.update_in(cx, |this, window, cx| this.save_to(&path, window, cx))
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
            self.reload_provider();
        }
        self.run_file_saved(path, window, cx);
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

pub fn key_bindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("secondary-q", Quit, None),
        KeyBinding::new("secondary-o", Open, None),
        KeyBinding::new("secondary-s", Save, None),
        KeyBinding::new("secondary-shift-s", SaveAs, None),
        KeyBinding::new("secondary-n", NewFile, None),
        KeyBinding::new("secondary-w", CloseTab, None),
        KeyBinding::new("secondary-shift-w", CloseWindow, None),
        KeyBinding::new("secondary-shift-]", NextTab, None),
        KeyBinding::new("secondary-shift-[", PreviousTab, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
        KeyBinding::new("secondary-k", OpenCommand, None),
        KeyBinding::new("secondary-shift-k", AddCommand, None),
        KeyBinding::new("secondary-b", ToggleSidebar, None),
        KeyBinding::new("secondary-shift-e", FocusFileTree, None),
        KeyBinding::new("secondary-p", GoToFile, None),
        KeyBinding::new("secondary-shift-f", FindInFiles, None),
        // As in IntelliJ on the Mac.
        KeyBinding::new("ctrl-r", RunSelected, None),
        KeyBinding::new("ctrl-alt-r", ChooseRunConfiguration, None),
        KeyBinding::new("secondary-f2", StopRun, None),
        KeyBinding::new("secondary-j", ToggleRunPanel, None),
        // In the editor too, where GPUI Kit binds ⇧⌘F to Replace; Replace
        // moves to ⌘R, as in IntelliJ.
        KeyBinding::new("secondary-shift-f", FindInFiles, Some("Input")),
        KeyBinding::new("secondary-r", Replace, Some("Input")),
        // Cmd+click does the same.
        KeyBinding::new("f12", GoToDefinition, Some("Input")),
    ]
    .into_iter()
    // Scoped to the workspace so that forms using Cmd+digits keep them.
    .chain(
        (1..=9)
            .map(|n| KeyBinding::new(&format!("secondary-{n}"), ActivateTab(n - 1), Some(CONTEXT))),
    )
    .chain(crate::file_tree::key_bindings())
    .chain(crate::find_in_files::key_bindings())
    .chain(jig_commands::new_command::key_bindings())
    .collect()
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let viewport = window.viewport_size();
        let theme = cx.theme();
        let sidebar = self.render_sidebar(cx);
        let sidebar_top = self.render_sidebar_top(cx);
        let title = self.render_title(cx);
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
            .capture_action(cx.listener(Self::on_escape))
            .capture_action(cx.listener(Self::on_accept_enter))
            .capture_action(cx.listener(Self::on_accept_tab))
            .capture_action(cx.listener(Self::on_shift_tab))
            .capture_action(cx.listener(Self::on_accept_indent))
            .capture_action(cx.listener(Self::on_undo))
            .on_drag_move(cx.listener(Self::drag_agent_chat))
            .on_drag_move(cx.listener(Self::resize_agent_chat))
            .child(
                // One unified bar across the window: the sidebar's top runs
                // up under the traffic lights, the rest holds the title or tabs.
                TitleBar::new()
                    .h(px(TITLE_BAR_HEIGHT))
                    .pl_0()
                    .border_b_0()
                    .bg(transparent_black())
                    .child(
                        h_flex().size_full().children(sidebar_top).child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .when(self.sidebar_shown(), |this| this.pl_2())
                                .when(!self.sidebar_shown(), |this| {
                                    this.pl(px(TRAFFIC_LIGHTS_WIDTH))
                                })
                                .pr_2()
                                .bg(theme::editor_surface(cx))
                                .border_b_1()
                                .border_color(theme.title_bar_border)
                                .child(title)
                                .children(self.render_command_button(cx))
                                .children(self.render_run_controls(cx)),
                        ),
                    ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .children(sidebar)
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .bg(theme::editor_surface(cx))
                            .when(self.home, |this| {
                                this.child(div().flex_1().min_h_0().child(self.render_start(cx)))
                            })
                            .when(!self.home, |this| {
                                this.child(
                                    div().flex_1().min_h_0().pl_2().pr_3().pt_1().pb_2().child(
                                        Editor::new(self.editor().state())
                                            .bordered(false)
                                            // The column behind it is the surface.
                                            .bg(transparent_black())
                                            // Locked while a command's change awaits
                                            // review. The element re-applies this
                                            // every frame.
                                            .readonly(self.previewing())
                                            .size_full(),
                                    ),
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
            .children(self.render_run_picker(window))
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
            window.press("secondary-down", cx);
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
            window.press("secondary-down", cx);
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
        let commands = dir.path().join("config").join("commands.toml");
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
        std::fs::write(&commands, "[[command]]\nname = \"Mine\"\nprompt = \"p\"\n").unwrap();
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
                    "\n[[command]]\nname = \"From file\"\nprompt = \"p\"\n",
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
            "[[command]]\nname = \"Create model\"\nscope = \"cursor\"\nprompt = \"Insert a model.\"\ncomment = \"required\"\n",
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
        cx.update(|_| {
            if let Ok(server) = crate::agent::server() {
                server.stop();
            }
        });
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
        std::fs::write(
            dir.path().join(".jig/run.toml"),
            r#"
[[run]]
name = "Other"
command = "true"

[[run]]
name = "Hello"
command = "echo hello $GREETING; echo 'src/lib.rs:1:4: here' >&2; exit 2"
env = { GREETING = "there" }
"#,
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
}
