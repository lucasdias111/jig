//! Jig: a code editor where AI works through small commands at the cursor.

// A windowed app on Windows, so no console window opens beside it. Debug
// builds keep theirs, for what Jig prints there. Pipes an agent hands
// `Jig --tasks-mcp` still reach it either way.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod about_window;
mod agent;
mod agents;
mod brackets;
mod branch_picker;
mod completions;
mod dap;
mod debug_target;
mod debuggers;
mod definitions;
mod diff;
mod document;
mod editorconfig;
mod field_box;
mod file_icons;
mod file_tree;
mod find_in_files;
mod find_panel;
mod formatters;
mod fuzzy;
mod git;
mod git_panel;
mod hover;
mod indentation;
mod languages;
mod lines;
mod lsp;
mod lsp_debug;
mod menus;
mod planner;
mod project;
mod project_search;
mod providers;
mod quick_open;
mod recent;
mod run_configs;
mod run_output;
mod run_picker;
mod session;
mod settings;
mod settings_window;
mod shortcuts;
mod snippets;
mod text_edits;
mod theme;
mod tools;
mod update;
mod windows;
mod workspace;

use std::path::PathBuf;

use gpui_kit::component::TitleBar;
use gpui_kit::*;
use workspace::{Quit, Workspace};

fn main() {
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == jig_ai::tasks::mcp::FLAG)
    {
        let args: Vec<String> = std::env::args_os()
            .skip(2)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        serve_tasks(&args);
        return;
    }
    if std::env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == jig_ai::map::FLAG)
    {
        let args: Vec<String> = std::env::args_os()
            .skip(2)
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        serve_map(&args);
        return;
    }
    // Agents start Jig itself as their task tool.
    if let Ok(executable) = std::env::current_exe() {
        jig_ai::tasks::mcp::set_command(executable);
    }

    let mut paths = std::env::args_os().skip(1).map(PathBuf::from);
    let path = paths.next();
    // Any further files open in tabs of their own.
    let more: Vec<PathBuf> = paths.collect();

    #[cfg(feature = "snapshots")]
    if let Some(dir) = std::env::var_os("JIG_SNAPSHOTS") {
        settings_window::render_snapshots(&PathBuf::from(dir));
        return;
    }

    // GPUI Kit's icons (chevrons, search, menus) load from its asset source;
    // without it they draw as nothing.
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            languages::register_grammars();
            let settings_error = settings::init(cx);
            providers::init(cx);
            recent::init(cx);
            session::init(cx);
            agent::init(cx);
            update::init(cx);
            theme::init(cx);
            theme::apply(None, cx);
            cx.bind_keys(workspace::key_bindings());
            // Before the menus: they show only shortcuts bound by then.
            windows::init(cx, is_editor_window);
            settings_window::init(cx);
            menus::init(cx);
            // Last: it takes over the shortcuts Settings can change.
            shortcuts::init(cx);
            // Reached only when no window handles Quit (e.g. none is open).
            cx.on_action(|_: &Quit, cx| cx.quit());
            // Settings and About can't open a file, so they don't keep Jig
            // running.
            cx.on_window_closed(|cx, _| {
                let windows = cx.windows();
                if !windows.iter().any(|window| is_editor_window(*window, cx)) {
                    cx.quit();
                }
            })
            .detach();

            let window = open_window(path, &more, cx).expect("failed to open window");
            cx.activate(true);
            if let Some(error) = settings_error {
                window
                    .update(cx, |_, window, cx| {
                        let detail = format!("Using the defaults until it's fixed. {error}");
                        // Nothing to do with the answer.
                        drop(window.prompt(
                            PromptLevel::Warning,
                            "Your settings couldn't be read",
                            Some(&detail),
                            &["OK"],
                            cx,
                        ));
                    })
                    .ok();
            }
        });
}

/// Whether `window` is one of Jig's editor windows, not Settings or About.
pub fn is_editor_window(window: AnyWindowHandle, cx: &App) -> bool {
    !settings_window::is_settings_window(window, cx) && !about_window::is_about_window(window, cx)
}

/// Open a Jig window on `path` (a file, a folder or nothing), with `more`
/// files in tabs of their own.
pub fn open_window(
    path: Option<PathBuf>,
    more: &[PathBuf],
    cx: &mut App,
) -> Result<WindowHandle<gpui_kit::base::Root>> {
    // A transparent titlebar: the workspace draws its own, so only the
    // traffic lights and the code remain. On macOS the window is
    // translucent so the sidebar picks up the desktop's vibrancy, and
    // the editor as much as the translucency setting lets it.
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1080.), px(760.)), cx)),
        window_min_size: Some(size(px(480.), px(320.))),
        titlebar: Some(TitlebarOptions {
            // Centred in the workspace's taller title bar.
            traffic_light_position: Some(point(px(16.), px(16.))),
            ..TitleBar::title_bar_options()
        }),
        window_background: if cfg!(target_os = "macos") {
            WindowBackgroundAppearance::Blurred
        } else {
            WindowBackgroundAppearance::Opaque
        },
        ..TitleBar::window_options()
    };
    cx.open_window(options, |window, cx| {
        let workspace = cx.new(|cx| {
            let mut workspace = Workspace::new(path, window, cx);
            for path in more {
                workspace.open_file(path, window, cx);
            }
            workspace
        });
        // The root would paint the theme's background over the
        // whole window, hiding the blur behind the sidebar and editor.
        cx.new(|cx| gpui_kit::base::Root::new(workspace, window, cx).bg(transparent_black()))
    })
}

/// `Jig --tasks-mcp`: the task planner as an agent's tool, on stdio, until
/// the agent hangs up.
/// `Jig --map-mcp`: the Map's drawing tools, for an agent.
fn serve_map(args: &[String]) {
    let served = jig_map::project_from_args(args).and_then(|project| {
        jig_map::serve_drawing_tools(&project, std::io::stdin().lock(), std::io::stdout().lock())
    });
    if let Err(error) = served {
        eprintln!("jig: {error:#}");
        std::process::exit(1);
    }
}

fn serve_tasks(args: &[String]) {
    let served = jig_ai::tasks::mcp::project_from_args(args).and_then(|project| {
        jig_ai::tasks::mcp::serve(&project, std::io::stdin().lock(), std::io::stdout().lock())
    });
    if let Err(error) = served {
        eprintln!("jig: {error:#}");
        std::process::exit(1);
    }
}
